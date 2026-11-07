"use client";

import { useCallback, useEffect, useMemo, useState } from "react";
import { Activity, RefreshCw, Search } from "lucide-react";

import { AppHeader } from "@/components/shell/app-header";
import { QueryRecordTraceDialog } from "@/components/plugins/query-record-trace";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { WEBUI } from "@/lib/i18n";
import { useI18n } from "@/lib/i18n/provider";
import {
  fetchQueryRecordDetail,
  fetchQueryRecords,
  type QueryRecordDetail,
  type QueryRecordRow,
} from "@/lib/oxidns-api";
import { useAppStore } from "@/lib/store";
import { cn } from "@/lib/utils";

type StatusFilter = "all" | "error" | "has_response";
type SourceRecord = QueryRecordRow & { recorderTag: string };

const EMPTY_FILTERS = {
  qname: "",
  client: "",
  status: "all" as StatusFilter,
};

export default function QueriesPage() {
  const { t, formatDateTime } = useI18n();
  const plugins = useAppStore((state) => state.plugins);
  const dependencyGraph = useAppStore((state) => state.dependencyGraph);

  const recorderTags = useMemo(
    () =>
      plugins
        .filter((plugin) => plugin.pluginKind === "query_recorder")
        .map((plugin) => plugin.name),
    [plugins],
  );

  const [records, setRecords] = useState<SourceRecord[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [qname, setQname] = useState("");
  const [client, setClient] = useState("");
  const [status, setStatus] = useState<StatusFilter>("all");
  const [recorder, setRecorder] = useState("all");
  const [applied, setApplied] = useState(EMPTY_FILTERS);
  const [refreshKey, setRefreshKey] = useState(0);
  const [selected, setSelected] = useState<QueryRecordDetail | null>(null);
  const [selectedTag, setSelectedTag] = useState<string | null>(null);
  const [detailLoading, setDetailLoading] = useState(false);

  const load = useCallback(async () => {
    const tags =
      recorder === "all"
        ? recorderTags
        : recorderTags.filter((tag) => tag === recorder);
    if (tags.length === 0) {
      setRecords([]);
      setLoading(false);
      return;
    }

    setLoading(true);
    setError(null);
    const results = await Promise.allSettled(
      tags.map(async (tag) => {
        const response = await fetchQueryRecords(tag, {
          limit: 100,
          qname: applied.qname.trim() || undefined,
          clientIp: applied.client.trim() || undefined,
          status: applied.status,
        });
        return response.records.map((record) => ({
          ...record,
          recorderTag: tag,
        }));
      }),
    );

    const merged: SourceRecord[] = [];
    let failed = 0;
    for (const result of results) {
      if (result.status === "fulfilled") {
        merged.push(...result.value);
      } else {
        failed += 1;
      }
    }
    merged.sort(
      (a, b) =>
        b.created_at_ms - a.created_at_ms ||
        b.id - a.id ||
        a.recorderTag.localeCompare(b.recorderTag),
    );
    setRecords(merged);
    if (failed === results.length && results.length > 0) {
      setError(t(WEBUI.queries.loadFailed));
    }
    setLoading(false);
  }, [applied, recorder, recorderTags, t]);

  useEffect(() => {
    const timer = window.setTimeout(() => {
      void load();
    }, 0);
    return () => window.clearTimeout(timer);
  }, [load, refreshKey]);

  const openDetail = useCallback(
    async (record: SourceRecord) => {
      setDetailLoading(true);
      try {
        const detail = await fetchQueryRecordDetail(
          record.recorderTag,
          record.id,
        );
        setSelected(detail.record);
        setSelectedTag(record.recorderTag);
      } catch {
        setError(t(WEBUI.queries.detailFailed));
      } finally {
        setDetailLoading(false);
      }
    },
    [t],
  );

  const hasRecorder = recorderTags.length > 0;

  return (
    <div className="flex h-full min-h-0 flex-col overflow-hidden">
      <AppHeader title={t(WEBUI.queries.title)} />
      <div className="flex min-h-0 flex-1 flex-col">
        <div className="space-y-3 border-b px-4 py-3">
          <div className="flex flex-wrap items-start gap-2">
            <div className="min-w-0 flex-1">
              <div className="flex items-center gap-2 text-sm font-medium">
                <Activity className="size-4 text-primary" />
                {t(WEBUI.queries.title)}
              </div>
              <p className="mt-1 text-xs text-muted-foreground">
                {t(WEBUI.queries.description)}
              </p>
            </div>
            <Button
              type="button"
              variant="outline"
              size="sm"
              onClick={() => setRefreshKey((value) => value + 1)}
              disabled={loading || !hasRecorder}
            >
              <RefreshCw
                className={cn("size-3.5", loading && "animate-spin")}
              />
              {t(WEBUI.queries.refresh)}
            </Button>
          </div>

          <form
            className="grid gap-2 sm:grid-cols-2 xl:grid-cols-[minmax(0,1fr)_minmax(0,1fr)_9rem_11rem_auto]"
            onSubmit={(event) => {
              event.preventDefault();
              setApplied({ qname, client, status });
            }}
          >
            <Input
              value={qname}
              onChange={(event) => setQname(event.target.value)}
              placeholder={t(WEBUI.queries.qnameFilter)}
              aria-label={t(WEBUI.queries.qnameFilter)}
            />
            <Input
              value={client}
              onChange={(event) => setClient(event.target.value)}
              placeholder={t(WEBUI.queries.clientFilter)}
              aria-label={t(WEBUI.queries.clientFilter)}
            />
            <select
              value={status}
              onChange={(event) =>
                setStatus(event.target.value as StatusFilter)
              }
              aria-label={t(WEBUI.queries.statusFilter)}
              className="h-8 rounded-md border bg-background px-2 text-sm"
            >
              <option value="all">{t(WEBUI.queries.allStatuses)}</option>
              <option value="has_response">
                {t(WEBUI.queries.statusHasResponse)}
              </option>
              <option value="error">{t(WEBUI.queries.statusError)}</option>
            </select>
            <select
              value={recorder}
              onChange={(event) => setRecorder(event.target.value)}
              aria-label={t(WEBUI.queries.recorderFilter)}
              className="h-8 rounded-md border bg-background px-2 text-sm"
            >
              <option value="all">{t(WEBUI.queries.allRecorders)}</option>
              {recorderTags.map((tag) => (
                <option key={tag} value={tag}>
                  {tag}
                </option>
              ))}
            </select>
            <div className="flex items-center gap-2">
              <Button type="submit" size="sm" disabled={!hasRecorder}>
                <Search className="size-3.5" />
                {t(WEBUI.queries.apply)}
              </Button>
              <Button
                type="button"
                variant="ghost"
                size="sm"
                onClick={() => {
                  setQname("");
                  setClient("");
                  setStatus("all");
                  setRecorder("all");
                  setApplied(EMPTY_FILTERS);
                }}
              >
                {t(WEBUI.queries.clear)}
              </Button>
            </div>
          </form>

          <div className="flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
            <span>
              {t(WEBUI.queries.recordsCount, { count: records.length })}
            </span>
            {detailLoading && <span>{t(WEBUI.common.loading)}</span>}
            {error && <span className="text-destructive">{error}</span>}
          </div>
        </div>

        <div className="min-h-0 flex-1 overflow-auto p-4">
          {!hasRecorder ? (
            <EmptyState
              title={t(WEBUI.queries.noRecorder)}
              hint={t(WEBUI.queries.noRecorderHint)}
            />
          ) : loading && records.length === 0 ? (
            <EmptyState title={t(WEBUI.queries.loading)} />
          ) : records.length === 0 ? (
            <EmptyState title={t(WEBUI.queries.empty)} />
          ) : (
            <div className="overflow-x-auto rounded-lg border">
              <Table className="min-w-[62rem]">
                <TableHeader>
                  <TableRow className="bg-muted/30 hover:bg-muted/30">
                    <TableHead>{t(WEBUI.queries.timeColumn)}</TableHead>
                    <TableHead>{t(WEBUI.queries.recorderColumn)}</TableHead>
                    <TableHead>{t(WEBUI.queries.serverColumn)}</TableHead>
                    <TableHead>{t(WEBUI.queries.clientColumn)}</TableHead>
                    <TableHead>{t(WEBUI.queries.queryColumn)}</TableHead>
                    <TableHead>{t(WEBUI.queries.resultColumn)}</TableHead>
                    <TableHead>{t(WEBUI.queries.elapsedColumn)}</TableHead>
                    <TableHead>{t(WEBUI.queries.responderColumn)}</TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {records.map((record) => (
                    <TableRow
                      key={`${record.recorderTag}:${record.id}`}
                      className="cursor-pointer"
                      onClick={() => void openDetail(record)}
                    >
                      <TableCell className="whitespace-nowrap font-mono text-xs">
                        {formatDateTime(record.created_at_ms, {
                          hour: "2-digit",
                          minute: "2-digit",
                          second: "2-digit",
                        })}
                      </TableCell>
                      <TableCell className="font-mono text-xs">
                        {record.recorderTag}
                      </TableCell>
                      <TableCell className="whitespace-nowrap text-xs">
                        <span className="font-mono">
                          {record.server_tag ?? "-"}
                        </span>
                        {record.protocol && (
                          <span className="ml-1 text-muted-foreground">
                            {record.protocol}
                          </span>
                        )}
                      </TableCell>
                      <TableCell className="whitespace-nowrap font-mono text-xs">
                        {record.client_ip}
                      </TableCell>
                      <TableCell className="max-w-[18rem]">
                        <div className="flex min-w-0 items-center gap-1.5">
                          <span className="truncate font-mono text-xs">
                            {questionName(record)}
                          </span>
                          <Badge
                            variant="secondary"
                            className="font-mono text-[10px]"
                          >
                            {questionType(record)}
                          </Badge>
                        </div>
                      </TableCell>
                      <TableCell>
                        <ResultBadge record={record} />
                      </TableCell>
                      <TableCell className="whitespace-nowrap font-mono text-xs">
                        {record.elapsed_ms}ms
                      </TableCell>
                      <TableCell className="max-w-[14rem] truncate text-xs">
                        {record.responder_tag ? (
                          <span className="font-mono">
                            {record.responder_tag}
                            {record.responder_upstream
                              ? ` · ${record.responder_upstream}`
                              : ""}
                          </span>
                        ) : (
                          <span className="text-muted-foreground">-</span>
                        )}
                      </TableCell>
                    </TableRow>
                  ))}
                </TableBody>
              </Table>
            </div>
          )}
        </div>
      </div>

      <QueryRecordTraceDialog
        record={selected}
        recorderTag={selectedTag ?? undefined}
        graph={dependencyGraph ?? null}
        onClose={() => {
          setSelected(null);
          setSelectedTag(null);
        }}
      />
    </div>
  );
}

function EmptyState({ title, hint }: { title: string; hint?: string }) {
  return (
    <div className="flex min-h-64 flex-col items-center justify-center gap-2 rounded-lg border border-dashed bg-muted/10 px-6 py-12 text-center">
      <Activity className="size-6 text-muted-foreground" />
      <div className="text-sm font-medium">{title}</div>
      {hint && (
        <div className="max-w-lg text-xs text-muted-foreground">{hint}</div>
      )}
    </div>
  );
}

function ResultBadge({ record }: { record: QueryRecordRow }) {
  if (record.error) {
    return <Badge variant="destructive">ERR</Badge>;
  }
  if (!record.has_response) {
    return <Badge variant="outline">-</Badge>;
  }
  return (
    <div className="flex flex-wrap items-center gap-1">
      <Badge variant="secondary" className="font-mono text-[10px]">
        {record.rcode ?? "-"}
      </Badge>
      <span className="font-mono text-[10px] text-muted-foreground">
        {record.answer_count}/{record.authority_count}/{record.additional_count}
      </span>
    </div>
  );
}

function questionName(record: QueryRecordRow): string {
  return record.questions_json?.[0]?.name ?? "-";
}

function questionType(record: QueryRecordRow): string {
  return record.questions_json?.[0]?.qtype ?? "-";
}
