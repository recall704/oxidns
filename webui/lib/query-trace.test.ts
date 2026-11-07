import { describe, expect, it } from "vitest";

import type {
  DependencyGraphReport,
  QueryRecordDetail,
  SequenceFlowExpression,
} from "@/lib/oxidns-api";
import { buildTrace } from "@/lib/query-trace";

function expr(
  raw: string,
  extra: Partial<SequenceFlowExpression> = {},
): SequenceFlowExpression {
  return {
    field: "args[0]",
    raw,
    kind: "plugin",
    inverted: false,
    ...extra,
  };
}

function graph(): DependencyGraphReport {
  return {
    nodes: [],
    edges: [],
    init_order: [],
    sequence_flows: [
      {
        tag: "main",
        rules: [
          { index: 0, matches: [], exec: expr("$ttl") },
          {
            index: 1,
            matches: [expr("$qname suffix:corp.example")],
            exec: expr("$forward_local"),
          },
          {
            index: 2,
            matches: [
              expr("qname corp", {
                kind: "quick_setup",
                plugin_type: "qname",
                param: "corp",
              }),
              expr("client_ip 10.0.0.0/8", {
                kind: "quick_setup",
                plugin_type: "client_ip",
                param: "10.0.0.0/8",
              }),
            ],
            exec: expr("$forward_local"),
          },
          { index: 3, matches: [expr("$qtype A")], exec: expr("$ttl") },
        ],
      },
    ],
  };
}

function record(steps: QueryRecordDetail["steps"]): QueryRecordDetail {
  return {
    id: 1842,
    created_at_ms: 0,
    elapsed_ms: 18,
    request_id: 1,
    client_ip: "10.0.0.23",
    questions_json: [{ name: "api.corp.example.", qtype: "A", qclass: "IN" }],
    has_response: true,
    rcode: "NoError",
    answer_count: 2,
    authority_count: 1,
    additional_count: 1,
    answers_json: [],
    authorities_json: [],
    additionals_json: [],
    signature_json: [],
    server_tag: "udp_main",
    protocol: "udp",
    listen: "0.0.0.0:53",
    entry_tag: "main",
    responder_tag: "forward_local",
    responder_kind: "forward",
    responder_upstream: "223.5.5.5:53",
    responder_elapsed_ms: 18,
    steps,
  };
}

describe("buildTrace", () => {
  it("builds the executed path, evidence, responder and unreached rules", () => {
    const trace = buildTrace(
      record([
        {
          event_index: 0,
          sequence_tag: "main",
          node_index: 1,
          kind: "matcher",
          tag: "qs.match.main.1.0.qname",
          outcome: "not_matched",
          depth: 2,
          at_us: 100,
          detail: { observed: "api.corp.example." },
        },
        {
          event_index: 1,
          sequence_tag: "main",
          node_index: 2,
          kind: "matcher",
          tag: "qs.match.main.2.0.qname",
          outcome: "matched",
          depth: 2,
          at_us: 200,
          detail: { observed: "api.corp.example." },
        },
        {
          event_index: 2,
          sequence_tag: "main",
          node_index: 2,
          kind: "matcher",
          tag: "qs.match.main.2.1.client_ip",
          outcome: "matched",
          depth: 2,
          at_us: 300,
          detail: { observed: "10.0.0.23" },
        },
        {
          event_index: 3,
          sequence_tag: "main",
          node_index: 2,
          kind: "executor",
          tag: "forward_local",
          outcome: "entered",
          depth: 2,
          at_us: 400,
        },
        {
          event_index: 4,
          sequence_tag: "main",
          node_index: 2,
          kind: "resolver",
          tag: "223.5.5.5:53",
          outcome: "stop",
          depth: 3,
          at_us: 18400,
          detail: { source: "223.5.5.5:53", elapsed_ms: 18 },
        },
        {
          event_index: 5,
          sequence_tag: "main",
          node_index: 2,
          kind: "executor",
          tag: "forward_local",
          outcome: "stop",
          depth: 2,
          at_us: 18500,
          detail: { source: "223.5.5.5:53", note: "forward" },
        },
      ]),
      graph(),
    );

    const rules = trace.rows.filter((row) => row.kind === "rule");
    expect(rules.map((row) => row.title)).toEqual(["#1", "#2", "#3"]);
    expect(rules[0].status).toBe("skip");
    expect(rules[0].matchers?.[0].label).toBe("$qname suffix:corp.example");
    expect(rules[1].status).toBe("ok");
    expect(rules[1].matchers?.map((matcher) => matcher.label)).toEqual([
      "quick(qname) corp",
      "quick(client_ip) 10.0.0.0/8",
    ]);
    expect(rules[1].matchers?.[0].observed).toBe("api.corp.example.");
    expect(rules[1].action?.tag).toBe("forward_local");
    expect(rules[2].unreached).toBe(true);

    expect(trace.rows.some((row) => row.kind === "resolver")).toBe(true);
    expect(trace.rows.find((row) => row.kind === "sequence")?.title).toBe(
      "main",
    );
    expect(trace.responder).toEqual({
      tag: "forward_local",
      kind: "forward",
      upstream: "223.5.5.5:53",
      elapsedMs: 18,
    });
    expect(trace.stats).toMatchObject({
      steps: 6,
      executedRules: 1,
      skippedRules: 1,
      unreachedRules: 1,
      upstreamMs: 18,
    });
  });

  it("falls back to flat depth when old records have no annotations", () => {
    const trace = buildTrace(
      record([
        {
          event_index: 0,
          sequence_tag: "main",
          node_index: 0,
          kind: "executor",
          tag: "hosts",
          outcome: "stop",
        },
      ]),
      graph(),
    );

    expect(trace.rows.map((row) => row.kind)).toEqual([
      "server",
      "sequence",
      "rule",
      "rule",
      "rule",
      "rule",
    ]);
    expect(trace.rows.every((row) => row.depth === 0)).toBe(true);
  });
});
