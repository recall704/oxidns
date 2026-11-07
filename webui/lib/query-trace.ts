import type {
  DependencyGraphReport,
  QueryRecordDetail,
  QueryRecorderStep,
  QueryRecorderStepDetail,
  SequenceFlowExpression,
  SequenceFlowReport,
} from "@/lib/oxidns-api";

export type TraceRowKind =
  | "server"
  | "sequence"
  | "rule"
  | "executor"
  | "builtin"
  | "resolver";

export type TraceStatus = "ok" | "skip" | "run" | "error" | "neutral";

export interface TraceMatcher {
  tag: string;
  label?: string;
  outcome: string;
  observed?: string;
  status: TraceStatus;
}

export interface TraceAction {
  tag: string;
  label?: string;
  outcome: string;
  detail?: QueryRecorderStepDetail | null;
}

export interface TraceRow {
  id: string;
  depth: number;
  kind: TraceRowKind;
  title: string;
  subtitle?: string;
  outcome?: string;
  status: TraceStatus;
  durationMs?: number;
  matchers?: TraceMatcher[];
  action?: TraceAction;
  detail?: QueryRecorderStepDetail | null;
  ruleIndex?: number;
  sequenceTag?: string;
  unreached?: boolean;
  startedAtUs?: number;
  endedAtUs?: number;
}

export interface TraceResponder {
  tag: string;
  kind: string;
  upstream?: string;
  elapsedMs?: number;
}

export interface TraceStats {
  steps: number;
  executedRules: number;
  skippedRules: number;
  unreachedRules: number;
  upstreamMs: number;
}

export interface TraceModel {
  rows: TraceRow[];
  responder: TraceResponder | null;
  stats: TraceStats;
  serverTag?: string;
  protocol?: string;
  listen?: string;
  entryTag?: string;
}

interface RuleState {
  rowIndex: number;
  closed: boolean;
  occurrence: number;
}

const MATCHED_OUTCOMES = new Set([
  "matched",
  "always_true_matched",
  "always_false_matched",
]);

const SKIPPED_OUTCOMES = new Set([
  "not_matched",
  "always_true_not_matched",
  "always_false_not_matched",
]);

export function buildTrace(
  record: QueryRecordDetail,
  graph: DependencyGraphReport | null,
  recorderTag?: string,
): TraceModel {
  const steps = [...(record.steps ?? [])].sort(
    (a, b) => a.event_index - b.event_index,
  );
  const depthOffset = steps.length
    ? Math.max((steps[0]?.depth ?? 0) - 2, 0)
    : 0;
  const depthOf = (value?: number) => Math.max((value ?? 0) - depthOffset, 0);
  const flows = new Map<string, SequenceFlowReport>(
    (graph?.sequence_flows ?? []).map((flow) => [flow.tag, flow]),
  );
  const rows: TraceRow[] = [];
  const ruleStates = new Map<string, RuleState>();
  let activeSequence: string | undefined;
  let previousDepth = 0;
  let previousTag: string | undefined;

  const serverTag = record.server_tag ?? undefined;
  const protocol = record.protocol ?? undefined;
  const listen = record.listen ?? undefined;
  const entryTag = record.entry_tag ?? undefined;

  rows.push({
    id: "server",
    depth: 0,
    kind: "server",
    title: serverTag ?? "server",
    subtitle: [protocol, listen].filter(Boolean).join(" · ") || undefined,
    status: "neutral",
    detail: {
      note: entryTag ? `entry ${entryTag}` : undefined,
    },
  });

  for (const step of steps) {
    const sequenceTag = step.sequence_tag || previousTag || "";
    const depth = depthOf(step.depth);

    if (recorderTag && step.kind === "executor" && step.tag === recorderTag) {
      continue;
    }

    if (sequenceTag && sequenceTag !== activeSequence) {
      const returning = depth < previousDepth;
      rows.push({
        id: `seq:${sequenceTag}:${rows.length}`,
        depth: Math.max(depth - 1, 0),
        kind: "sequence",
        title: returning ? `↩ ${sequenceTag}` : sequenceTag,
        subtitle:
          !returning && sequenceTag === entryTag
            ? "entry"
            : returning
              ? "return"
              : undefined,
        status: "neutral",
        sequenceTag,
      });
      activeSequence = sequenceTag;
    }
    previousDepth = depth;
    previousTag = sequenceTag;

    if (step.kind === "matcher") {
      const row = ensureRuleRow(rows, ruleStates, sequenceTag, step, depth);
      const rule = ruleStates.get(ruleKey(sequenceTag, step.node_index));
      const matcherIndex =
        rule === undefined ? 0 : (rows[row.rowIndex].matchers?.length ?? 0);
      const expression = ruleExpression(
        flows.get(sequenceTag),
        step.node_index,
        matcherIndex,
      );
      rows[row.rowIndex].matchers = [
        ...(rows[row.rowIndex].matchers ?? []),
        {
          tag: step.tag ?? "-",
          label: expressionLabel(expression),
          outcome: step.outcome,
          observed: step.detail?.observed ?? undefined,
          status: matcherStatus(step.outcome),
        },
      ];
      rows[row.rowIndex].startedAtUs ??= step.at_us ?? 0;
      rows[row.rowIndex].endedAtUs = step.at_us ?? 0;
      updateRuleStatus(rows[row.rowIndex]);
      continue;
    }

    if (step.kind === "resolver") {
      rows.push({
        id: `resolver:${step.event_index}`,
        depth,
        kind: "resolver",
        title: step.detail?.source ?? step.tag ?? "resolver",
        subtitle: step.detail?.note ?? undefined,
        outcome: step.outcome,
        status: "ok",
        durationMs: step.detail?.elapsed_ms ?? undefined,
        detail: step.detail,
        sequenceTag,
      });
      continue;
    }

    if (step.kind === "executor" || step.kind === "builtin") {
      if (step.outcome === "entered") {
        continue;
      }
      const row = ensureRuleRow(rows, ruleStates, sequenceTag, step, depth);
      const target = rows[row.rowIndex];
      target.action = {
        tag: step.tag ?? step.kind,
        label: actionLabel(flows.get(sequenceTag), step.node_index, step.tag),
        outcome: step.outcome,
        detail: step.detail,
      };
      target.endedAtUs = step.at_us ?? target.endedAtUs ?? 0;
      target.durationMs = deriveDuration(target, step);
      target.status =
        step.outcome === "error"
          ? "error"
          : step.outcome === "stop" || step.outcome === "return"
            ? "ok"
            : "run";
      ruleStates.get(ruleKey(sequenceTag, step.node_index))!.closed = true;
    }
  }

  appendUnreachedRules(rows, flows);

  const executedRules = rows.filter(
    (row) => row.kind === "rule" && !row.unreached && row.action,
  ).length;
  const skippedRules = rows.filter(
    (row) => row.kind === "rule" && !row.unreached && !row.action,
  ).length;
  const unreachedRules = rows.filter((row) => row.unreached).length;
  const upstreamMs =
    record.responder_elapsed_ms ??
    rows.find((row) => row.kind === "resolver")?.durationMs ??
    0;

  return {
    rows,
    responder: responderFromRecord(record),
    stats: {
      steps: steps.length,
      executedRules,
      skippedRules,
      unreachedRules,
      upstreamMs,
    },
    serverTag,
    protocol,
    listen,
    entryTag,
  };
}

export function expressionLabel(
  expression?: SequenceFlowExpression,
): string | undefined {
  if (!expression) return undefined;
  const prefix = expression.inverted ? "!" : "";
  if (expression.kind === "builtin") {
    const param = expression.param ? ` ${expression.param}` : "";
    return `${expression.builtin ?? "builtin"}${param}`;
  }
  if (expression.target_tag) {
    return `${prefix}$${expression.target_tag}`;
  }
  if (expression.kind === "quick_setup") {
    const param = expression.param ? ` ${expression.param}` : "";
    return `${prefix}quick(${expression.plugin_type ?? "?"})${param}`;
  }
  return `${prefix}${expression.raw}`;
}

export function matcherStatus(outcome: string): TraceStatus {
  if (MATCHED_OUTCOMES.has(outcome)) return "ok";
  if (SKIPPED_OUTCOMES.has(outcome)) return "skip";
  return "neutral";
}

export function responderKindLabel(kind?: string | null): string | undefined {
  if (!kind) return undefined;
  const labels: Record<string, string> = {
    executor: "executor",
    cache: "cache",
    hosts: "hosts",
    forward: "forward",
    synthetic: "synthetic",
    server_fallback: "server fallback",
    server_error: "server error",
  };
  return labels[kind] ?? kind;
}

function ruleKey(sequenceTag: string, nodeIndex?: number): string {
  return `${sequenceTag}:${nodeIndex ?? -1}`;
}

function ensureRuleRow(
  rows: TraceRow[],
  ruleStates: Map<string, RuleState>,
  sequenceTag: string,
  step: QueryRecorderStep,
  depth: number,
): RuleState & { rowIndex: number } {
  const key = ruleKey(sequenceTag, step.node_index);
  const existing = ruleStates.get(key);
  if (existing && !existing.closed) {
    return existing as RuleState & { rowIndex: number };
  }

  const occurrence = (existing?.occurrence ?? 0) + 1;
  const index = rows.length;
  rows.push({
    id: `rule:${key}:${occurrence}`,
    depth,
    kind: "rule",
    title: typeof step.node_index === "number" ? `#${step.node_index}` : "#?",
    status: "neutral",
    ruleIndex: step.node_index,
    sequenceTag,
    matchers: [],
  });
  const state = { rowIndex: index, closed: false, occurrence };
  ruleStates.set(key, state);
  return state as RuleState & { rowIndex: number };
}

function updateRuleStatus(row: TraceRow) {
  const matchers = row.matchers ?? [];
  if (matchers.some((matcher) => matcher.status === "skip")) {
    row.status = "skip";
  } else if (row.action) {
    row.status = row.action.outcome === "error" ? "error" : "run";
  }
}

function ruleExpression(
  flow: SequenceFlowReport | undefined,
  nodeIndex: number | undefined,
  matcherIndex: number,
): SequenceFlowExpression | undefined {
  if (!flow || typeof nodeIndex !== "number") return undefined;
  return flow.rules[nodeIndex]?.matches[matcherIndex];
}

function actionLabel(
  flow: SequenceFlowReport | undefined,
  nodeIndex: number | undefined,
  runtimeTag: string | undefined,
): string | undefined {
  if (flow && typeof nodeIndex === "number") {
    const label = expressionLabel(flow.rules[nodeIndex]?.exec);
    if (label) return label;
  }
  return runtimeTag;
}

function deriveDuration(
  row: TraceRow,
  step: QueryRecorderStep,
): number | undefined {
  if (typeof step.detail?.elapsed_ms === "number") {
    return step.detail.elapsed_ms;
  }
  if (
    typeof row.startedAtUs === "number" &&
    typeof row.endedAtUs === "number"
  ) {
    return Math.max(0, (row.endedAtUs - row.startedAtUs) / 1000);
  }
  return undefined;
}

function appendUnreachedRules(
  rows: TraceRow[],
  flows: Map<string, SequenceFlowReport>,
) {
  const lastIndexBySequence = new Map<string, number>();
  rows.forEach((row, index) => {
    if (row.sequenceTag) {
      lastIndexBySequence.set(row.sequenceTag, index);
    }
  });

  for (const [sequenceTag, lastIndex] of [
    ...lastIndexBySequence.entries(),
  ].sort((a, b) => b[1] - a[1])) {
    const flow = flows.get(sequenceTag);
    if (!flow) continue;
    const executed = rows
      .filter(
        (row) =>
          row.kind === "rule" &&
          row.sequenceTag === sequenceTag &&
          !row.unreached,
      )
      .map((row) => row.ruleIndex ?? -1);
    const lastExecuted = executed.length ? Math.max(...executed) : -1;
    if (lastExecuted < 0) continue;

    const lastRow = rows.find(
      (row) =>
        row.kind === "rule" &&
        row.sequenceTag === sequenceTag &&
        row.ruleIndex === lastExecuted &&
        !row.unreached,
    );
    const terminal = lastRow?.action?.outcome;
    if (terminal !== "stop" && terminal !== "return") continue;

    const depth = lastRow?.depth ?? 0;
    const unreached = flow.rules.filter((rule) => rule.index > lastExecuted);
    const inserts = unreached.map<TraceRow>((rule) => ({
      id: `unreached:${sequenceTag}:${rule.index}`,
      depth,
      kind: "rule",
      title: `#${rule.index}`,
      status: "neutral",
      ruleIndex: rule.index,
      sequenceTag,
      unreached: true,
      matchers: [],
      action: rule.exec
        ? {
            tag: "not executed",
            label: expressionLabel(rule.exec),
            outcome: "not_executed",
          }
        : undefined,
    }));
    rows.splice(lastIndex + 1, 0, ...inserts);
  }
}

function responderFromRecord(record: QueryRecordDetail): TraceResponder | null {
  if (!record.responder_tag) return null;
  return {
    tag: record.responder_tag,
    kind: responderKindLabel(record.responder_kind) ?? "unknown",
    upstream: record.responder_upstream ?? undefined,
    elapsedMs: record.responder_elapsed_ms ?? undefined,
  };
}

/* ------------------------------------------------------------------------- */
/* Topology                                                                  */
/* ------------------------------------------------------------------------- */

export interface TopologyRule {
  index: number;
  matchers: SequenceFlowExpression[];
  exec?: SequenceFlowExpression;
  status: TraceStatus;
  action?: string;
  unreached?: boolean;
}

export interface TopologySequence {
  tag: string;
  entry: boolean;
  rules: TopologyRule[];
  executed: boolean;
}

export function buildTopology(
  trace: TraceModel,
  graph: DependencyGraphReport | null,
  recorderTag?: string,
): TopologySequence[] {
  const flows = new Map<string, SequenceFlowReport>(
    (graph?.sequence_flows ?? []).map((flow) => [flow.tag, flow]),
  );
  const seen = new Set<string>();
  const sequences: TopologySequence[] = [];

  for (const row of trace.rows) {
    if (!row.sequenceTag || seen.has(row.sequenceTag)) continue;
    seen.add(row.sequenceTag);
    const flow = flows.get(row.sequenceTag);
    const rules: TopologyRule[] =
      flow?.rules
        .filter((rule) => !recorderTag || rule.exec?.target_tag !== recorderTag)
        .map((rule) => {
          const runtime = trace.rows.find(
            (candidate) =>
              candidate.kind === "rule" &&
              candidate.sequenceTag === row.sequenceTag &&
              candidate.ruleIndex === rule.index &&
              !candidate.unreached,
          );
          const unreached = trace.rows.some(
            (candidate) =>
              candidate.unreached &&
              candidate.sequenceTag === row.sequenceTag &&
              candidate.ruleIndex === rule.index,
          );
          return {
            index: rule.index,
            matchers: rule.matches,
            exec: rule.exec,
            status: runtime?.status ?? "neutral",
            action: runtime?.action?.tag,
            unreached,
          };
        }) ?? [];
    sequences.push({
      tag: row.sequenceTag,
      entry: row.sequenceTag === trace.entryTag,
      rules,
      executed: true,
    });
  }

  return sequences;
}
