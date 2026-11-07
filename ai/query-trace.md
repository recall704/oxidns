# Per-Request Query Trace

This document defines the per-request trace that powers the WebUI "request
detail" view: the ordered path from the inbound server to the responder, the
rules that matched, and the response that was returned.

The view is intentionally request-scoped. Aggregate query statistics remain in
the existing query recorder insights views.

## Goals

- Explain one request without reading debug logs:
  - which inbound server and entry accepted it;
  - which sequence rules matched, were skipped, or were never reached;
  - which executor produced the response and, for forwarding, which upstream
    won;
  - what the response contained.
- Keep the trace opt-in. No trace work is performed unless a `query_recorder`
  executor enables request-local path recording.
- Preserve the existing shared-trace deduplication. Per-request evidence and
  timings must not become part of the shared path fingerprint.

## Non-goals

- A global always-on trace sink. The recorder remains an executor and therefore
  only observes requests that flow through its continuation.
- Raw DNS wire bytes. The recorder stores decoded message snapshots.
- Full matcher evidence for every provider-backed rule. The first iteration
  records the observed query name, type, class, client address, and marks; a
  provider may add richer evidence later.

## Trace model

A recorded request contains:

```text
ingress     server tag, protocol, listen address, entry tag
steps       ordered trace events with depth, microsecond offset, detail
responder   executor tag, origin kind, upstream address, elapsed time
response    existing decoded response snapshot
```

SNI and DoH URL paths remain request metadata for executors; revision 2 does
not persist them.

### Ingress

`IngressContext` gains typed identity fields populated by the server plugin
before the entry executor runs:

- `server_tag`: the server plugin tag (`udp_main`, `doh_main`, ...);
- `protocol`: `udp`, `tcp`, `dot`, `doh`, `doq`, `doh3`;
- `listen`: the configured listen address;
- `entry`: the entry executor tag;
- the existing SNI / URL path metadata.

The fields are attached per request by `RequestHandle`, which already owns the
server identity. No per-request allocation is introduced; the values are
`Arc<str>` / `SocketAddr` copies.

### Steps

Each `ExecutionPathEvent` gains:

| field | meaning |
| --- | --- |
| `depth` | visual tree depth, maintained by scoped entry |
| `at_us` | microseconds since the recorder started the path |
| `detail` | optional matcher evidence, jump/goto target, resolver result |

Depth rules:

- request root (server row) is rendered by the client at depth 0;
- a sequence invocation enters one scope;
- a rule instruction enters one scope, so its matcher/action events are one
  level below the sequence;
- a `with_next` executor enters one scope around the downstream continuation;
- a forwarding resolver exchange enters one scope for the upstream event.

The client renders the tree from `depth` directly. The depth is capped at the
fourth visible level; deeper nesting keeps the same indentation and is marked
with a continuation glyph.

Matcher evidence is optional. The matcher trait exposes `evidence(&DnsContext)`
with a default `None`. The first implementations cover `qname`, `qtype`,
`qclass`, `client_ip`, and `mark`. Provider-backed rules still report the
observed value even when they cannot name the exact provider entry.

Control-flow events record their target:

- `jump <sequence>` / `goto <sequence>`: `detail.target = <sequence tag>`;
- `reject <rcode>`: `detail.note = <rcode>`;
- forwarding: a `resolver` event with `detail.source = <upstream address>` and
  `detail.elapsed_ms`.

### Responder

`ResponseOrigin` is set by the executor that produces a response:

```text
executor_tag, kind, upstream, elapsed_ms
```

Kinds: `executor`, `cache`, `hosts`, `forward`, `synthetic`, `server_fallback`,
`server_error`.

The chain sets `executor` only when a response appears where there was none, so
the first producer wins. Forward executors overwrite it with `forward` plus the
winning upstream. Cache/hosts/synthetic can annotate their own kind without
changing the response-production rule.

If no executor produces a response, the server synthesizes an empty `NOERROR`
or `SERVFAIL` after the recorder continuation has already returned. That
response is outside the trace; the record has `has_response = false` and the UI
labels it as a server-side fallback.

## Storage

`query_recorder` keeps the existing `v2` table prefix and bumps the
`schema_revision` in the `meta` table from `1` to `2`.

Record table additions (nullable, per record):

```text
server_tag, protocol, listen, entry_tag,
responder_tag, responder_kind, responder_upstream, responder_elapsed_ms
```

Step additions are **not** stored in `trace_steps`, because that table is shared
by records with the same structural path. Instead, per-record annotations are
stored in the compressed record payload:

```text
step_annotations[]: { event_index, depth, at_us, detail }
```

The shared trace fingerprint continues to use only the structural step fields
(`event_index`, `sequence_tag`, `node_index`, `kind`, `tag`, `outcome`).
Loading a record detail merges the shared steps with the record annotations by
`event_index`. Old payloads decode with an empty annotation list and render as a
flat list with no timing.

Migration from revision `1` uses `ALTER TABLE ... ADD COLUMN` with `NULL`
defaults. Unknown revisions continue to be rejected.

## API

`GET /plugins/{tag}/records/{id}` keeps its shape and adds:

- the ingress/responder fields on `record`;
- `depth`, `at_us`, and `detail` on each `steps[]` item.

No new endpoint is required. The SSE stream emits the same detail shape.

## WebUI

A top-level **Query records** page (left navigation, at the same level as
settings) merges recent records from every enabled `query_recorder` plugin.
Operators can filter by QNAME, client IP, status, and recorder, then open a
record in the detail shell described below. The page links to the same pure
trace builder and detail component as the plugin-level record list.

The query record detail becomes a dedicated shell with three tabs:

1. **Path** (default): a vertical tree with 4ch indentation per level. It shows
   the server, entry wrapper when present, sequence invocations, rules,
   matchers, actions, and resolver exchanges. Skipped rules show the first
   failing matcher; unreached rules are inferred from the previous `stop` and
   rendered dimmed. The client normalizes absolute depth against the first
   recorded step and hides the recorder's own executor step, so placing the
   recorder inside a sequence does not shift or pollute the tree.
2. **Response content**: RCODE, flags, section counts, questions, answers,
   authorities, additionals, signatures, and EDNS.
3. **Topology**: the configured sequence/rule graph with the executed path
   highlighted and unconsumed branches dimmed. Server, sequence, and responder
   terminals are included so the graph ends at the response.

A shared footer shows the responder, upstream, response summary, and a time bar
on every tab.

Indentation is a single CSS variable: `--trace-indent: 4ch` on desktop and
`3ch` below 560px. Levels beyond the fourth use the same indent plus a
continuation marker.

## Coverage

The recorder only sees its own continuation. Place it as the first rule of the
entry sequence, or before the rules it should explain. An entry executor runs
without a continuation, so configuring the recorder directly as a server
`entry` records no downstream rules.

The runtime does not enforce this; the UI can flag a recorder whose configured
reach does not cover every server entry, and `oxidns check` can report the same
condition through the dependency graph.

```yaml
plugins:
  - tag: recorder
    type: query_recorder
    args:
      path: "./data/query-recorder.db"

  - tag: main
    type: sequence
    args:
      - exec: $recorder          # first rule: wraps everything below it
      - matches: qname corp.example
        exec: $forward
```


## Performance

- Path recording is disabled by default; scoped entry/exit and event pushes are
  no-ops when disabled.
- When enabled, each event costs one monotonic clock read and one small
  allocation for the event.
- Ingress identity is copied once per request; no formatting or lookup occurs
  on the hot path.
- Responder and resolver detail are only recorded while the path is enabled.

## Testing

- Core: scope depth for sequences, rules, `with_next`, `jump`/`goto`, and
  resolver events; no events when the path is disabled; timestamps are
  monotonic.
- Chain: first-response origin attribution, jump/goto target detail, matcher
  short-circuit.
- Forward: winning upstream and elapsed time reach the resolver event; single
  and concurrent paths.
- Recorder: revision 1 to 2 migration, shared trace deduplication unchanged,
  annotation round-trip, old payload fallback, record columns round-trip.
- WebUI: pure trace-tree builder tests for skipped/unreached rules, nested
  sequences, missing annotations, and responder fallbacks; i18n coverage; light
  and dark layouts at desktop and 360px widths.

## Rollout

1. Runtime trace fields and ingress/responder attribution.
2. Recorder schema revision 2, payload annotations, and API fields.
3. WebUI detail shell and the three tabs.
4. Coverage diagnostics and provider-specific evidence.
