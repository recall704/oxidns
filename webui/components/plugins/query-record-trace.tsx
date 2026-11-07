"use client";

import type { ReactNode } from "react";
import { useMemo } from "react";
import { ArrowRight, GitBranch, Server, Waypoints } from "lucide-react";

import { Badge } from "@/components/ui/badge";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { WEBUI } from "@/lib/i18n";
import { useI18n } from "@/lib/i18n/provider";
import { cn } from "@/lib/utils";
import type {
  DependencyGraphReport,
  QueryRecordDetail,
  QueryRecordPayload,
} from "@/lib/oxidns-api";
import {
  buildTopology,
  buildTrace,
  expressionLabel,
  type TraceModel,
  type TraceRow,
} from "@/lib/query-trace";

export function QueryRecordTraceDialog({
  record,
  graph,
  recorderTag,
  onClose,
}: {
  record: QueryRecordDetail | null;
  graph: DependencyGraphReport | null;
  recorderTag?: string;
  onClose: () => void;
}) {
  const { t, formatDateTime } = useI18n();
  const trace = useMemo(
    () => (record ? buildTrace(record, graph, recorderTag) : null),
    [record, graph, recorderTag],
  );

  if (!record || !trace) return null;

  const question = record.questions_json?.[0];
  return (
    <Dialog open onOpenChange={(open) => !open && onClose()}>
      <DialogContent className="flex max-h-[88vh] flex-col gap-0 overflow-hidden p-0 sm:max-w-5xl">
        <DialogHeader className="border-b px-4 pt-5 pr-12 pb-3">
          <DialogTitle className="flex flex-wrap items-center gap-2">
            <span>{t(WEBUI.queryTrace.detailTitle, { id: record.id })}</span>
            <Badge variant="outline" className="font-mono">
              {record.rcode ?? "-"}
            </Badge>
            <span className="text-xs font-normal text-muted-foreground">
              {formatDateTime(record.created_at_ms, {
                dateStyle: "medium",
                timeStyle: "medium",
              })}
            </span>
          </DialogTitle>
          <DialogDescription className="flex flex-wrap items-center gap-1.5 pt-1 text-xs">
            {question && (
              <span className="font-mono text-foreground">
                {question.name} {question.qtype}
              </span>
            )}
            <span className="text-muted-foreground">·</span>
            <span className="font-mono">{record.client_ip}</span>
            <span className="text-muted-foreground">·</span>
            <span className="font-mono">{record.elapsed_ms}ms</span>
          </DialogDescription>
        </DialogHeader>

        <Tabs
          defaultValue="path"
          className="flex min-h-0 flex-1 flex-col gap-0"
        >
          <div className="border-b px-4">
            <TabsList variant="line" className="h-10 w-full justify-start">
              <TabsTrigger value="path">
                {t(WEBUI.queryTrace.pathTab)}
                <span className="ml-1 font-mono text-[10px] text-muted-foreground">
                  {trace.stats.steps}
                </span>
              </TabsTrigger>
              <TabsTrigger value="response">
                {t(WEBUI.queryTrace.responseTab)}
              </TabsTrigger>
              <TabsTrigger value="topology">
                {t(WEBUI.queryTrace.topologyTab)}
              </TabsTrigger>
            </TabsList>
          </div>

          <div className="min-h-0 flex-1 overflow-y-auto">
            <TabsContent value="path" className="m-0 p-4">
              <QueryRecordPath record={record} trace={trace} />
            </TabsContent>
            <TabsContent value="response" className="m-0 p-4">
              <QueryRecordResponse record={record} />
            </TabsContent>
            <TabsContent value="topology" className="m-0 p-4">
              <QueryRecordTopology
                record={record}
                trace={trace}
                graph={graph}
                recorderTag={recorderTag}
              />
            </TabsContent>
          </div>

          <QueryRecordFooter record={record} trace={trace} />
        </Tabs>
      </DialogContent>
    </Dialog>
  );
}

function QueryRecordPath({
  record,
  trace,
}: {
  record: QueryRecordDetail;
  trace: TraceModel;
}) {
  const { t } = useI18n();
  if (trace.rows.length <= 1) {
    return (
      <div className="rounded-md border border-dashed bg-muted/10 px-3 py-8 text-center text-sm text-muted-foreground">
        {t(WEBUI.queryTrace.noSteps)}
      </div>
    );
  }
  return (
    <div className="space-y-3 [--trace-indent:3ch] sm:[--trace-indent:4ch]">
      <TraceBreadcrumb record={record} trace={trace} />
      <div className="flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
        <span>
          {t(WEBUI.queryTrace.pathSummary, {
            steps: trace.stats.steps,
            executed: trace.stats.executedRules,
            skipped: trace.stats.skippedRules,
            unreached: trace.stats.unreachedRules,
          })}
        </span>
        <span className="ml-auto inline-flex items-center gap-1">
          <span className="size-1.5 rounded-full bg-primary" />
          {t(WEBUI.queryRecordFlow.matched)}
        </span>
        <span className="inline-flex items-center gap-1">
          <span className="size-1.5 rounded-full bg-destructive" />
          {t(WEBUI.queryRecordFlow.notMatched)}
        </span>
        <span className="inline-flex items-center gap-1">
          <span className="size-1.5 rounded-full bg-muted-foreground/40" />
          {t(WEBUI.queryTrace.unreached)}
        </span>
      </div>

      <div className="space-y-0.5">
        {trace.rows.map((row) => (
          <TraceRowView key={row.id} row={row} />
        ))}
      </div>
    </div>
  );
}

function TraceBreadcrumb({
  record,
  trace,
}: {
  record: QueryRecordDetail;
  trace: TraceModel;
}) {
  const sequences = trace.rows
    .filter((row) => row.kind === "sequence" && !row.title.startsWith("↩"))
    .map((row) => row.title);
  const executed = trace.rows.filter(
    (row) => row.kind === "rule" && row.action && !row.unreached,
  );
  const last = executed[executed.length - 1];
  const segments = [
    trace.serverTag,
    trace.entryTag && trace.entryTag !== sequences[0]
      ? trace.entryTag
      : undefined,
    ...sequences,
    last ? `${last.title} ${last.action?.tag ?? ""}`.trim() : undefined,
    trace.responder?.upstream ?? trace.responder?.tag,
    record.rcode ?? undefined,
  ].filter((value): value is string => Boolean(value));

  if (segments.length === 0) return null;
  return (
    <div className="flex flex-wrap items-center gap-1 rounded-md border bg-muted/20 px-2.5 py-1.5 text-[11px]">
      {segments.map((segment, index) => (
        <span key={`${segment}-${index}`} className="flex items-center gap-1">
          {index > 0 && (
            <ArrowRight className="size-3 text-muted-foreground/60" />
          )}
          <span className="font-mono">{segment}</span>
        </span>
      ))}
    </div>
  );
}

function TraceRowView({ row }: { row: TraceRow }) {
  const { t } = useI18n();
  const depth = Math.min(row.depth, 4);
  const indent = `calc(${depth} * var(--trace-indent))`;

  return (
    <div
      className={cn(
        "relative rounded-md py-1.5 pr-2",
        row.kind === "rule" && row.status === "ok" && "bg-primary/5",
        row.kind === "rule" && row.status === "skip" && "bg-destructive/5",
        row.unreached && "opacity-55",
        row.kind === "sequence" && "mt-2",
      )}
      style={{
        paddingLeft: `calc(${depth} * var(--trace-indent) + 0.5rem)`,
      }}
    >
      {depth > 0 && (
        <span
          aria-hidden
          className="absolute inset-y-0 border-l border-border/60"
          style={{ left: `calc(${indent} - 0.55rem)` }}
        />
      )}
      <div className="flex min-w-0 items-start gap-2">
        <span
          className={cn(
            "mt-1.5 size-2 shrink-0 rounded-full",
            row.status === "ok" && "bg-primary",
            row.status === "skip" && "bg-destructive",
            row.status === "run" && "bg-sky-500",
            row.status === "error" && "bg-destructive",
            row.status === "neutral" && "bg-muted-foreground/35",
          )}
        />
        <div className="min-w-0 flex-1">
          <div className="flex min-w-0 flex-wrap items-center gap-1.5">
            {row.depth > 4 && (
              <span className="font-mono text-[10px] text-muted-foreground">
                └
              </span>
            )}
            <KindLabel kind={row.kind} />
            <span
              className={cn(
                "min-w-0 truncate font-mono text-xs font-semibold",
                row.kind === "rule" && "text-sm",
              )}
            >
              {row.title}
            </span>
            {row.subtitle && (
              <span className="truncate text-[11px] text-muted-foreground">
                {row.subtitle}
              </span>
            )}
            {row.kind === "rule" && (
              <RuleStatus status={row.status} unreached={row.unreached} />
            )}
            {typeof row.durationMs === "number" && (
              <span className="font-mono text-[11px] text-muted-foreground">
                {formatMs(row.durationMs)}
              </span>
            )}
          </div>

          {row.kind === "rule" && (
            <div className="mt-1 flex flex-wrap items-center gap-1.5">
              {(row.matchers ?? []).length === 0 ? (
                <span className="rounded border border-primary/30 bg-primary/10 px-1.5 py-0.5 text-[10px] text-primary">
                  {t(WEBUI.queryRecordFlow.alwaysMatch)}
                </span>
              ) : (
                row.matchers?.map((matcher, index) => (
                  <span
                    key={`${matcher.tag}-${index}`}
                    className={cn(
                      "inline-flex max-w-full items-center gap-1 rounded border px-1.5 py-0.5 text-[10px]",
                      matcher.status === "ok" &&
                        "border-primary/30 bg-primary/10 text-primary",
                      matcher.status === "skip" &&
                        "border-destructive/30 bg-destructive/8 text-destructive",
                      matcher.status === "neutral" &&
                        "border-border bg-muted/30 text-muted-foreground",
                    )}
                  >
                    <span>{matcher.status === "skip" ? "×" : "✓"}</span>
                    <span className="min-w-0 truncate font-mono">
                      {matcher.label ?? matcher.tag}
                    </span>
                    {matcher.observed && (
                      <span className="max-w-40 truncate rounded border border-dashed px-1 text-muted-foreground">
                        {matcher.observed}
                      </span>
                    )}
                  </span>
                ))
              )}
              {row.action && (
                <>
                  <ArrowRight className="size-3 text-muted-foreground/60" />
                  <span
                    className={cn(
                      "inline-flex max-w-full items-center gap-1 rounded border px-1.5 py-0.5 text-[10px]",
                      row.action.outcome === "error"
                        ? "border-destructive/40 bg-destructive/10 text-destructive"
                        : "border-sky-500/30 bg-sky-500/10 text-sky-700 dark:text-sky-300",
                    )}
                  >
                    <span className="min-w-0 truncate font-mono">
                      {row.action.label ?? row.action.tag}
                    </span>
                    {row.action.outcome !== "not_executed" && (
                      <span className="rounded bg-background/70 px-1 font-mono text-[9px]">
                        {row.action.outcome}
                      </span>
                    )}
                  </span>
                </>
              )}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

function KindLabel({ kind }: { kind: TraceRow["kind"] }) {
  const { t } = useI18n();
  const labels: Record<TraceRow["kind"], string> = {
    server: t(WEBUI.queryTrace.server),
    sequence: t(WEBUI.queryTrace.sequence),
    rule: t(WEBUI.queryTrace.rule),
    executor: t(WEBUI.queryTrace.executor),
    builtin: t(WEBUI.queryTrace.builtin),
    resolver: t(WEBUI.queryTrace.resolver),
  };
  return (
    <span className="shrink-0 rounded border px-1 py-px text-[9px] uppercase tracking-wide text-muted-foreground">
      {labels[kind]}
    </span>
  );
}

function RuleStatus({
  status,
  unreached,
}: {
  status: TraceRow["status"];
  unreached?: boolean;
}) {
  const { t } = useI18n();
  if (unreached) {
    return (
      <span className="rounded-full border px-1.5 text-[10px] text-muted-foreground">
        {t(WEBUI.queryTrace.unreached)}
      </span>
    );
  }
  if (status === "skip") {
    return (
      <span className="rounded-full border border-destructive/30 px-1.5 text-[10px] text-destructive">
        {t(WEBUI.queryRecordFlow.notMatched)}
      </span>
    );
  }
  if (status === "ok") {
    return (
      <span className="rounded-full border border-primary/30 px-1.5 text-[10px] text-primary">
        {t(WEBUI.queryTrace.executed)}
      </span>
    );
  }
  if (status === "run") {
    return (
      <span className="rounded-full border border-sky-500/30 px-1.5 text-[10px] text-sky-700 dark:text-sky-300">
        {t(WEBUI.queryRecordFlow.next)}
      </span>
    );
  }
  return null;
}

function QueryRecordResponse({ record }: { record: QueryRecordDetail }) {
  const { t } = useI18n();
  if (!record.has_response) {
    return (
      <div className="space-y-2">
        {record.error && (
          <div className="rounded-md border border-destructive/30 bg-destructive/5 px-3 py-2 font-mono text-xs text-destructive">
            {record.error}
          </div>
        )}
        <div className="rounded-md border border-dashed bg-muted/10 px-3 py-8 text-center text-sm text-muted-foreground">
          {t(WEBUI.queryTrace.noResponseBody)}
        </div>
      </div>
    );
  }
  const edns = record.req_edns_json as
    | {
        udp_payload_size?: number;
        ext_rcode?: number;
        version?: number;
        dnssec_ok?: boolean;
      }
    | null
    | undefined;
  return (
    <div className="space-y-4">
      <div className="grid gap-2 sm:grid-cols-2 lg:grid-cols-4">
        <Metric label="RCODE" value={record.rcode ?? "-"} />
        <Metric
          label={t(WEBUI.queryRecorder.respFlagsLabel)}
          value={`AA=${flag(record.resp_aa)} TC=${flag(record.resp_tc)} RA=${flag(record.resp_ra)} AD=${flag(record.resp_ad)}`}
        />
        <Metric
          label={t(WEBUI.queryTrace.countLabel)}
          value={`${record.answer_count} / ${record.authority_count} / ${record.additional_count}`}
        />
        <Metric
          label={t(WEBUI.queryTrace.requestIdLabel)}
          value={String(record.request_id)}
        />
      </div>
      {edns && (
        <div className="rounded-md border border-dashed bg-muted/10 px-3 py-1.5 font-mono text-[11px] text-muted-foreground">
          EDNS udp {edns.udp_payload_size ?? "-"} · DO{" "}
          {edns.dnssec_ok ? "1" : "0"} · ext rcode {edns.ext_rcode ?? 0} ·
          version {edns.version ?? 0}
        </div>
      )}

      <ResponseSection
        title={t(WEBUI.dnsRecord.questionSection)}
        records={record.questions_json.map((question) => ({
          name: question.name,
          class: question.qclass,
          ttl: 0,
          rr_type: question.qtype,
          payload_kind: "",
          payload_text: "",
          payload: null,
        }))}
        hideTtl
        hideData
      />
      <ResponseSection
        title={t(WEBUI.dnsRecord.answerSection)}
        records={record.answers_json}
      />
      <ResponseSection
        title={t(WEBUI.dnsRecord.authoritySection)}
        records={record.authorities_json}
      />
      <ResponseSection
        title={t(WEBUI.dnsRecord.additionalSection)}
        records={record.additionals_json}
      />
      <ResponseSection
        title={t(WEBUI.dnsRecord.signatureSection)}
        records={record.signature_json}
      />
    </div>
  );
}

function Metric({ label, value }: { label: string; value: string }) {
  return (
    <div className="min-w-0 rounded-md border bg-muted/20 px-3 py-2">
      <div className="text-[11px] text-muted-foreground">{label}</div>
      <div className="mt-0.5 truncate font-mono text-xs">{value}</div>
    </div>
  );
}

function ResponseSection({
  title,
  records,
  hideTtl,
  hideData,
}: {
  title: string;
  records: QueryRecordPayload[];
  hideTtl?: boolean;
  hideData?: boolean;
}) {
  const { t } = useI18n();
  return (
    <section className="space-y-1.5">
      <div className="flex items-center gap-2 text-xs font-medium text-muted-foreground">
        <span>{title}</span>
        <Badge variant="secondary" className="h-4 px-1 font-mono text-[10px]">
          {records.length}
        </Badge>
      </div>
      {records.length === 0 ? (
        <div className="rounded-md border border-dashed bg-muted/10 px-3 py-2 text-xs text-muted-foreground">
          {t(WEBUI.dnsRecord.noRecords)}
        </div>
      ) : (
        <div className="overflow-x-auto rounded-md border">
          <table className="w-full min-w-[36rem] border-collapse text-xs">
            <thead>
              <tr className="border-b bg-muted/30 text-left text-muted-foreground">
                <th className="px-2 py-1.5 font-medium">name</th>
                <th className="px-2 py-1.5 font-medium">type</th>
                <th className="px-2 py-1.5 font-medium">class</th>
                {!hideTtl && (
                  <th className="px-2 py-1.5 text-right font-medium">TTL</th>
                )}
                {!hideData && <th className="px-2 py-1.5 font-medium">data</th>}
              </tr>
            </thead>
            <tbody>
              {records.map((record, index) => (
                <tr
                  key={`${record.name}-${record.rr_type}-${index}`}
                  className="border-b last:border-b-0"
                >
                  <td className="max-w-[16rem] truncate px-2 py-1.5 font-mono">
                    {record.name}
                  </td>
                  <td className="px-2 py-1.5">{record.rr_type}</td>
                  <td className="px-2 py-1.5">{record.class}</td>
                  {!hideTtl && (
                    <td className="px-2 py-1.5 text-right font-mono">
                      {record.ttl}
                    </td>
                  )}
                  {!hideData && (
                    <td className="max-w-[20rem] break-all px-2 py-1.5 font-mono text-muted-foreground">
                      {record.payload_text || record.payload_kind}
                    </td>
                  )}
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </section>
  );
}

function QueryRecordTopology({
  record,
  trace,
  graph,
  recorderTag,
}: {
  record: QueryRecordDetail;
  trace: TraceModel;
  graph: DependencyGraphReport | null;
  recorderTag?: string;
}) {
  const { t } = useI18n();
  const sequences = useMemo(
    () => buildTopology(trace, graph, recorderTag),
    [trace, graph, recorderTag],
  );

  if (!graph || sequences.length === 0) {
    return (
      <div className="rounded-md border border-dashed bg-muted/10 px-3 py-8 text-center text-sm text-muted-foreground">
        {t(WEBUI.queryRecordFlow.topologyUnavailable)}
      </div>
    );
  }

  return (
    <div className="space-y-3">
      <div className="text-xs text-muted-foreground">
        {t(WEBUI.queryTrace.topologySummary, {
          sequences: sequences.length,
          rules: sequences.reduce(
            (sum, sequence) => sum + sequence.rules.length,
            0,
          ),
        })}
      </div>
      <div className="flex flex-wrap items-stretch gap-2">
        <TopologyNode
          icon={<Server className="size-3.5" />}
          title={record.server_tag ?? "server"}
          subtitle={[record.protocol, record.listen]
            .filter(Boolean)
            .join(" · ")}
          tone="blue"
        />
        <Arrow />
        {sequences.map((sequence, index) => (
          <div key={sequence.tag} className="flex items-stretch gap-2">
            <div className="w-[18rem] max-w-full rounded-md border border-primary/40 border-l-4 bg-card">
              <div className="flex items-center justify-between gap-2 border-b px-2.5 py-1.5">
                <div className="flex items-center gap-1.5">
                  <GitBranch className="size-3.5 text-primary" />
                  <span className="font-mono text-xs font-semibold">
                    {sequence.tag}
                  </span>
                  {sequence.entry && (
                    <Badge variant="outline" className="h-4 px-1 text-[9px]">
                      entry
                    </Badge>
                  )}
                </div>
                <span className="text-[10px] text-muted-foreground">
                  {sequence.rules.length} rules
                </span>
              </div>
              <div>
                {sequence.rules.map((rule) => (
                  <div
                    key={`${sequence.tag}-${rule.index}`}
                    className={cn(
                      "grid grid-cols-[1.6rem_minmax(0,1fr)_auto] items-center gap-1.5 border-b px-2 py-1.5 last:border-b-0",
                      rule.status === "ok" && "bg-primary/5",
                      rule.status === "skip" && "bg-destructive/5",
                      rule.unreached && "opacity-50",
                    )}
                  >
                    <span className="font-mono text-[10px] text-muted-foreground">
                      #{rule.index}
                    </span>
                    <span className="flex min-w-0 flex-wrap gap-1">
                      {rule.matchers.length === 0 ? (
                        <span className="rounded border px-1 text-[9px] text-muted-foreground">
                          always
                        </span>
                      ) : (
                        rule.matchers.map((matcher, index) => (
                          <span
                            key={`${matcher.raw}-${index}`}
                            className="max-w-full truncate rounded border px-1 font-mono text-[9px]"
                          >
                            {expressionLabel(matcher)}
                          </span>
                        ))
                      )}
                    </span>
                    <span className="max-w-32 truncate font-mono text-[9px] text-muted-foreground">
                      {expressionLabel(rule.exec) ?? rule.action}
                    </span>
                  </div>
                ))}
              </div>
            </div>
            {index < sequences.length - 1 && <Arrow />}
          </div>
        ))}
        <Arrow />
        <TopologyNode
          icon={<Waypoints className="size-3.5" />}
          title={trace.responder?.tag ?? record.responder_tag ?? "-"}
          subtitle={[trace.responder?.kind, trace.responder?.upstream]
            .filter(Boolean)
            .join(" · ")}
          tone="primary"
        />
      </div>
      <div className="flex flex-wrap gap-3 text-[10px] text-muted-foreground">
        <span className="inline-flex items-center gap-1">
          <span className="size-1.5 rounded-full bg-primary" />
          {t(WEBUI.queryTrace.executed)}
        </span>
        <span className="inline-flex items-center gap-1">
          <span className="size-1.5 rounded-full bg-destructive" />
          {t(WEBUI.queryRecordFlow.notMatched)}
        </span>
        <span className="inline-flex items-center gap-1">
          <span className="size-1.5 rounded-full bg-muted-foreground/35" />
          {t(WEBUI.queryTrace.unreached)}
        </span>
      </div>
    </div>
  );
}

function TopologyNode({
  icon,
  title,
  subtitle,
  tone,
}: {
  icon: ReactNode;
  title: string;
  subtitle?: string;
  tone: "blue" | "primary";
}) {
  return (
    <div
      className={cn(
        "min-w-[9rem] max-w-[14rem] rounded-md border border-l-4 bg-card px-2.5 py-1.5",
        tone === "blue" ? "border-sky-500/40" : "border-primary/40",
      )}
    >
      <div className="flex items-center gap-1.5">
        {icon}
        <span className="truncate font-mono text-xs font-semibold">
          {title}
        </span>
      </div>
      {subtitle && (
        <div className="mt-0.5 truncate font-mono text-[10px] text-muted-foreground">
          {subtitle}
        </div>
      )}
    </div>
  );
}

function Arrow() {
  return (
    <div className="flex items-center text-muted-foreground">
      <ArrowRight className="size-3.5" />
    </div>
  );
}

function QueryRecordFooter({
  record,
  trace,
}: {
  record: QueryRecordDetail;
  trace: TraceModel;
}) {
  const { t } = useI18n();
  const total = Math.max(record.elapsed_ms, 1);
  const upstream = Math.min(trace.stats.upstreamMs, total);
  const other = Math.max(total - upstream, 0);
  const upstreamPct = (upstream / total) * 100;

  return (
    <div className="border-t bg-muted/20 px-4 py-2.5">
      <div className="flex flex-wrap items-center gap-2 text-xs">
        <Badge variant={trace.responder ? "secondary" : "outline"}>
          {t(WEBUI.queryTrace.responder)}
        </Badge>
        {trace.responder ? (
          <>
            <span className="font-mono font-medium">{trace.responder.tag}</span>
            {trace.responder.kind !== trace.responder.tag && (
              <span className="text-muted-foreground">
                {trace.responder.kind}
              </span>
            )}
            {trace.responder.upstream && (
              <span className="font-mono text-muted-foreground">
                {trace.responder.upstream}
              </span>
            )}
          </>
        ) : (
          <span className="text-muted-foreground">
            {t(WEBUI.queryTrace.noResponder)}
          </span>
        )}
        <span className="ml-auto flex items-center gap-2 font-mono text-muted-foreground">
          <span>{record.rcode ?? "-"}</span>
          <span>
            {record.answer_count} / {record.authority_count} /{" "}
            {record.additional_count}
          </span>
          <span>{formatMs(total)}</span>
        </span>
      </div>
      <div className="mt-2 flex h-1.5 overflow-hidden rounded-full border bg-muted">
        {upstream > 0 && (
          <span className="bg-primary" style={{ width: `${upstreamPct}%` }} />
        )}
        <span
          className="bg-sky-500/70"
          style={{ width: `${Math.max(100 - upstreamPct, 0)}%` }}
        />
      </div>
      <div className="mt-1 flex flex-wrap gap-3 text-[10px] text-muted-foreground">
        <span className="inline-flex items-center gap-1">
          <span className="size-1.5 rounded-full bg-primary" />
          {t(WEBUI.queryTrace.upstreamLabel)} {formatMs(upstream)}
        </span>
        <span className="inline-flex items-center gap-1">
          <span className="size-1.5 rounded-full bg-sky-500/70" />
          {t(WEBUI.queryTrace.otherLabel)} {formatMs(other)}
        </span>
      </div>
    </div>
  );
}

function formatMs(value: number): string {
  if (!Number.isFinite(value) || value <= 0) return "0ms";
  if (value < 1) return `${value.toFixed(2)}ms`;
  if (value < 10) return `${value.toFixed(1)}ms`;
  return `${Math.round(value)}ms`;
}

function flag(value: unknown): string {
  if (typeof value !== "boolean") return "-";
  return value ? "1" : "0";
}
