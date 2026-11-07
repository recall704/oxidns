"use client";

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  CheckCircle2,
  Database,
  Globe2,
  Loader2,
  Network,
  Pencil,
  Plus,
  RefreshCw,
  Save,
  Search,
  Trash2,
  X,
} from "lucide-react";

import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogMedia,
  AlertDialogTitle,
  AlertDialogTrigger,
} from "@/components/ui/alert-dialog";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from "@/components/ui/dialog";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { Textarea } from "@/components/ui/textarea";
import {
  RuleDbUnavailableError,
  addRuleDbRules,
  clearRuleDbRules,
  deleteRuleDbRules,
  fetchRuleDbRules,
  updateRuleDbRule,
  type RuleCategoryCount,
  type RuleKind,
  type RuleRecord,
  type RuleStats,
} from "@/lib/oxidns-api";
import { WEBUI } from "@/lib/i18n";
import { useI18n } from "@/lib/i18n/provider";

const PAGE_LIMIT = 100;
const EMPTY_STATS: RuleStats = {
  total: 0,
  domain: 0,
  ip: 0,
  enabled: 0,
  disabled: 0,
};

const KNOWN_CATEGORIES = ["ad", "gfw", "china", "cdn"] as const;

const CATEGORY_TONES: Record<string, string> = {
  ad: "border-destructive/40 bg-destructive/10 text-destructive",
  gfw: "border-chart-4/40 bg-chart-4/10 text-chart-4",
  china: "border-chart-2/40 bg-chart-2/10 text-chart-2",
  cdn: "border-chart-3/40 bg-chart-3/10 text-chart-3",
  default: "border-border bg-muted/30 text-muted-foreground",
};

const CONFIG_EXAMPLE = `rule_db:
  path: data/rules.db
  import:
    - "&data/rules.txt"
    - "suffix.example.com"
  import_mode: if-empty`;

type LoadState = "loading" | "ready" | "unconfigured" | "error";
type KindFilter = RuleKind | "all";
type StateFilter = "all" | "enabled" | "disabled";
type DraftKind = RuleKind | "auto";

interface EditDraft {
  id: number;
  rule: string;
  kind: RuleKind;
  category: string;
  enabled: boolean;
}

export function RuleDbView() {
  const { t } = useI18n();
  const [state, setState] = useState<LoadState>("loading");
  const [rules, setRules] = useState<RuleRecord[]>([]);
  const [stats, setStats] = useState<RuleStats>(EMPTY_STATS);
  const [categoryStats, setCategoryStats] = useState<RuleCategoryCount[]>([]);
  const [total, setTotal] = useState(0);
  const [nextCursor, setNextCursor] = useState<number | null>(null);
  const [database, setDatabase] = useState("");
  const [loadingMore, setLoadingMore] = useState(false);

  const [search, setSearch] = useState("");
  const [appliedSearch, setAppliedSearch] = useState("");
  const [kindFilter, setKindFilter] = useState<KindFilter>("all");
  const [categoryFilter, setCategoryFilter] = useState<string>("all");
  const [stateFilter, setStateFilter] = useState<StateFilter>("all");

  const [draft, setDraft] = useState("");
  const [draftKind, setDraftKind] = useState<DraftKind>("auto");
  const [draftCategory, setDraftCategory] = useState<string>("default");
  const [addOpen, setAddOpen] = useState(false);
  const [adding, setAdding] = useState(false);
  const [clearing, setClearing] = useState(false);
  const [removingId, setRemovingId] = useState<number | null>(null);
  const [saving, setSaving] = useState(false);
  const [busyId, setBusyId] = useState<number | null>(null);
  const [editing, setEditing] = useState<EditDraft | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  const abortRef = useRef<AbortController | null>(null);

  useEffect(() => {
    const timer = window.setTimeout(() => {
      setAppliedSearch(search.trim());
    }, 300);
    return () => window.clearTimeout(timer);
  }, [search]);

  const load = useCallback(
    async (cursor?: number) => {
      abortRef.current?.abort();
      const controller = new AbortController();
      abortRef.current = controller;
      if (cursor === undefined) {
        setState((current) => (current === "ready" ? "ready" : "loading"));
      } else {
        setLoadingMore(true);
      }
      setError(null);
      try {
        const response = await fetchRuleDbRules({
          cursor,
          limit: PAGE_LIMIT,
          q: appliedSearch || undefined,
          kind: kindFilter,
          category: categoryFilter,
          enabled:
            stateFilter === "all"
              ? "all"
              : stateFilter === "enabled"
                ? "true"
                : "false",
          signal: controller.signal,
        });
        if (controller.signal.aborted) return;
        setState("ready");
        setDatabase(response.database);
        setStats(response.stats);
        setCategoryStats(response.categories);
        setTotal(response.total);
        setNextCursor(response.next_cursor ?? null);
        setRules((current) =>
          cursor === undefined
            ? response.rules
            : [...current, ...response.rules],
        );
      } catch (err) {
        if (controller.signal.aborted) return;
        if (err instanceof RuleDbUnavailableError) {
          setState("unconfigured");
          return;
        }
        setState((current) => (current === "ready" ? "ready" : "error"));
        setError(
          err instanceof Error ? err.message : t(WEBUI.ruleDb.readFailed),
        );
      } finally {
        if (abortRef.current === controller) {
          abortRef.current = null;
          setLoadingMore(false);
        }
      }
    },
    [appliedSearch, kindFilter, categoryFilter, stateFilter, t],
  );

  useEffect(() => {
    void load();
    return () => abortRef.current?.abort();
  }, [load]);

  const parseDraftRules = () =>
    draft
      .split(/\r?\n/)
      .map((line) => line.trim())
      .filter((line) => line.length > 0 && !line.startsWith("#"));

  const handleAdd = async () => {
    const lines = parseDraftRules();
    if (lines.length === 0) {
      setError(t(WEBUI.ruleDb.inputRequired));
      return;
    }
    setAdding(true);
    setError(null);
    setNotice(null);
    try {
      const response = await addRuleDbRules(
        lines.map((line) => ({
          rule: line,
          kind: draftKind === "auto" ? null : draftKind,
        })),
        "api",
        draftCategory,
      );
      setNotice(
        t(WEBUI.ruleDb.addNotice, {
          added: response.added,
          skipped: response.skipped,
          total: response.total,
        }),
      );
      setDraft("");
      setAddOpen(false);
      await load();
    } catch (err) {
      setError(err instanceof Error ? err.message : t(WEBUI.ruleDb.addFailed));
    } finally {
      setAdding(false);
    }
  };

  const handleToggle = async (rule: RuleRecord) => {
    setBusyId(rule.id);
    setError(null);
    setNotice(null);
    try {
      await updateRuleDbRule({ id: rule.id, enabled: !rule.enabled });
      await load();
    } catch (err) {
      setError(
        err instanceof Error ? err.message : t(WEBUI.ruleDb.toggleFailed),
      );
    } finally {
      setBusyId(null);
    }
  };

  const handleSaveEdit = async () => {
    if (!editing) return;
    setSaving(true);
    setError(null);
    setNotice(null);
    try {
      await updateRuleDbRule({
        id: editing.id,
        rule: editing.rule,
        kind: editing.kind,
        category: editing.category,
        enabled: editing.enabled,
      });
      setEditing(null);
      setNotice(t(WEBUI.ruleDb.editNotice));
      await load();
    } catch (err) {
      setError(err instanceof Error ? err.message : t(WEBUI.ruleDb.editFailed));
    } finally {
      setSaving(false);
    }
  };

  const handleDelete = async (id: number) => {
    setRemovingId(id);
    setError(null);
    setNotice(null);
    try {
      const response = await deleteRuleDbRules([id]);
      setNotice(t(WEBUI.ruleDb.deleteNotice, { removed: response.removed }));
      await load();
    } catch (err) {
      setError(
        err instanceof Error ? err.message : t(WEBUI.ruleDb.deleteFailed),
      );
    } finally {
      setRemovingId(null);
    }
  };

  const handleClear = async () => {
    setClearing(true);
    setError(null);
    setNotice(null);
    try {
      const response = await clearRuleDbRules();
      setRules([]);
      setTotal(0);
      setNextCursor(null);
      setStats(EMPTY_STATS);
      setCategoryStats([]);
      setNotice(t(WEBUI.ruleDb.clearNotice, { removed: response.removed }));
      await load();
    } catch (err) {
      setError(
        err instanceof Error ? err.message : t(WEBUI.ruleDb.clearFailed),
      );
    } finally {
      setClearing(false);
    }
  };

  const statsCards = useMemo(
    () => [
      {
        key: "total",
        label: t(WEBUI.ruleDb.statTotal),
        value: stats.total,
        icon: Database,
        tone: "text-primary",
      },
      {
        key: "domain",
        label: t(WEBUI.ruleDb.statDomain),
        value: stats.domain,
        icon: Globe2,
        tone: "text-chart-2",
      },
      {
        key: "ip",
        label: t(WEBUI.ruleDb.statIp),
        value: stats.ip,
        icon: Network,
        tone: "text-chart-3",
      },
      {
        key: "enabled",
        label: t(WEBUI.ruleDb.statEnabled),
        value: stats.enabled,
        icon: CheckCircle2,
        tone: "text-chart-4",
      },
    ],
    [stats, t],
  );

  const categoryLabel = useCallback(
    (category: string) => {
      switch (category) {
        case "ad":
          return t(WEBUI.ruleDb.categoryAd);
        case "gfw":
          return t(WEBUI.ruleDb.categoryGfw);
        case "china":
          return t(WEBUI.ruleDb.categoryChina);
        case "cdn":
          return t(WEBUI.ruleDb.categoryCdn);
        case "default":
          return t(WEBUI.ruleDb.categoryDefault);
        default:
          return category;
      }
    },
    [t],
  );

  const categoryOptions = useMemo(() => {
    const counts = new Map(
      categoryStats.map((entry) => [entry.category, entry.total]),
    );
    const ids: string[] = [...KNOWN_CATEGORIES];
    for (const entry of categoryStats) {
      if (!ids.includes(entry.category)) ids.push(entry.category);
    }
    if (!ids.includes("default")) ids.push("default");
    return ids.map((id) => ({
      id,
      label: categoryLabel(id),
      count: counts.get(id) ?? 0,
    }));
  }, [categoryStats, categoryLabel]);

  if (state === "unconfigured") {
    return (
      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2">
            <Database className="h-5 w-5" />
            {t(WEBUI.ruleDb.unconfiguredTitle)}
          </CardTitle>
          <CardDescription>{t(WEBUI.ruleDb.unconfiguredDesc)}</CardDescription>
        </CardHeader>
        <CardContent className="space-y-3">
          <div className="text-xs font-medium text-muted-foreground">
            {t(WEBUI.ruleDb.configExample)}
          </div>
          <pre className="overflow-x-auto rounded-md border bg-muted/40 p-3 text-xs leading-relaxed">
            <code>{CONFIG_EXAMPLE}</code>
          </pre>
        </CardContent>
      </Card>
    );
  }

  return (
    <div className="space-y-4">
      <div className="flex flex-wrap items-end justify-between gap-3">
        <div className="min-w-0">
          <h2 className="text-lg font-semibold">{t(WEBUI.ruleDb.title)}</h2>
          <p className="mt-1 text-sm text-muted-foreground">
            {t(WEBUI.ruleDb.subtitle)}
          </p>
          {database && (
            <p className="mt-2 flex items-center gap-1.5 font-mono text-xs text-muted-foreground">
              <Database className="h-3.5 w-3.5" />
              <span className="truncate">{database}</span>
            </p>
          )}
        </div>
        <div className="flex items-center gap-2">
          <Dialog open={addOpen} onOpenChange={setAddOpen}>
            <DialogTrigger asChild>
              <Button size="sm">
                <Plus className="h-4 w-4" />
                {t(WEBUI.ruleDb.addButton)}
              </Button>
            </DialogTrigger>
            <DialogContent className="sm:max-w-xl">
              <DialogHeader>
                <DialogTitle>{t(WEBUI.ruleDb.addTitle)}</DialogTitle>
                <DialogDescription>{t(WEBUI.ruleDb.addDesc)}</DialogDescription>
              </DialogHeader>
              <div className="space-y-3">
                <Textarea
                  value={draft}
                  onChange={(event) => setDraft(event.target.value)}
                  disabled={adding}
                  placeholder={t(WEBUI.ruleDb.ruleInputPlaceholder)}
                  className="min-h-[160px] font-mono text-sm"
                />
                <div className="flex flex-wrap items-center gap-3 text-xs text-muted-foreground">
                  <div className="flex items-center gap-2">
                    <span>{t(WEBUI.ruleDb.parseKindLabel)}</span>
                    <Select
                      value={draftKind}
                      onValueChange={(value) =>
                        setDraftKind(value as DraftKind)
                      }
                      disabled={adding}
                    >
                      <SelectTrigger className="h-8 w-32">
                        <SelectValue />
                      </SelectTrigger>
                      <SelectContent>
                        <SelectItem value="auto">
                          {t(WEBUI.ruleDb.kindAuto)}
                        </SelectItem>
                        <SelectItem value="domain">
                          {t(WEBUI.ruleDb.kindDomain)}
                        </SelectItem>
                        <SelectItem value="ip">
                          {t(WEBUI.ruleDb.kindIp)}
                        </SelectItem>
                      </SelectContent>
                    </Select>
                  </div>
                  <div className="flex items-center gap-2">
                    <span>{t(WEBUI.ruleDb.filterCategoryLabel)}</span>
                    <Select
                      value={draftCategory}
                      onValueChange={setDraftCategory}
                      disabled={adding}
                    >
                      <SelectTrigger className="h-8 w-32">
                        <SelectValue />
                      </SelectTrigger>
                      <SelectContent>
                        {categoryOptions.map((option) => (
                          <SelectItem key={option.id} value={option.id}>
                            {option.label}
                          </SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                  </div>
                </div>
                <p className="text-xs text-muted-foreground">
                  {t(WEBUI.ruleDb.saveHint)}
                </p>
              </div>
              <DialogFooter>
                <Button
                  variant="outline"
                  disabled={adding}
                  onClick={() => setAddOpen(false)}
                >
                  {t(WEBUI.ruleDb.editCancel)}
                </Button>
                <Button onClick={() => void handleAdd()} disabled={adding}>
                  {adding ? (
                    <Loader2 className="h-4 w-4 animate-spin" />
                  ) : (
                    <Plus className="h-4 w-4" />
                  )}
                  {t(WEBUI.ruleDb.addButton)}
                </Button>
              </DialogFooter>
            </DialogContent>
          </Dialog>
          <Button
            variant="outline"
            size="sm"
            onClick={() => void load()}
            disabled={state === "loading"}
          >
            <RefreshCw className="h-4 w-4" />
            {t(WEBUI.ruleDb.refresh)}
          </Button>
          <AlertDialog>
            <AlertDialogTrigger asChild>
              <Button
                variant="outline"
                size="sm"
                disabled={clearing || stats.total === 0}
              >
                {clearing ? (
                  <Loader2 className="h-4 w-4 animate-spin" />
                ) : (
                  <Trash2 className="h-4 w-4 text-destructive" />
                )}
                {t(WEBUI.ruleDb.clearButton)}
              </Button>
            </AlertDialogTrigger>
            <AlertDialogContent>
              <AlertDialogHeader>
                <AlertDialogMedia className="bg-destructive/10 text-destructive">
                  <Trash2 className="h-5 w-5" />
                </AlertDialogMedia>
                <AlertDialogTitle>
                  {t(WEBUI.ruleDb.clearTitle)}
                </AlertDialogTitle>
                <AlertDialogDescription>
                  {t(WEBUI.ruleDb.clearDesc, { count: stats.total })}
                </AlertDialogDescription>
              </AlertDialogHeader>
              <AlertDialogFooter>
                <AlertDialogCancel disabled={clearing}>
                  {t(WEBUI.ruleDb.editCancel)}
                </AlertDialogCancel>
                <AlertDialogAction
                  variant="destructive"
                  disabled={clearing}
                  onClick={(event) => {
                    event.preventDefault();
                    void handleClear();
                  }}
                >
                  {t(WEBUI.ruleDb.clearButton)}
                </AlertDialogAction>
              </AlertDialogFooter>
            </AlertDialogContent>
          </AlertDialog>
        </div>
      </div>

      <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-4">
        {statsCards.map((card) => (
          <Card key={card.key}>
            <CardContent className="flex items-center justify-between gap-3 p-4">
              <div>
                <div className="text-xs text-muted-foreground">
                  {card.label}
                </div>
                <div className="mt-1 text-2xl font-semibold tabular-nums">
                  {card.value}
                </div>
              </div>
              <card.icon className={`h-8 w-8 ${card.tone}`} />
            </CardContent>
          </Card>
        ))}
      </div>

      <div className="flex flex-wrap gap-2">
        <Button
          variant={categoryFilter === "all" ? "default" : "outline"}
          size="sm"
          onClick={() => setCategoryFilter("all")}
        >
          {t(WEBUI.ruleDb.allCategories)}
          <Badge variant="outline" className="ml-1">
            {stats.total}
          </Badge>
        </Button>
        {categoryOptions.map((option) => (
          <Button
            key={option.id}
            variant={categoryFilter === option.id ? "default" : "outline"}
            size="sm"
            onClick={() => setCategoryFilter(option.id)}
          >
            {option.label}
            <Badge variant="outline" className="ml-1">
              {option.count}
            </Badge>
          </Button>
        ))}
      </div>

      <Card>
        <CardHeader className="gap-3 pb-2">
          <div className="flex flex-wrap items-center justify-between gap-2">
            <CardTitle className="text-sm">{t(WEBUI.ruleDb.title)}</CardTitle>
            <div className="flex flex-wrap gap-2 text-xs text-muted-foreground">
              <Badge variant="outline">
                {t(WEBUI.ruleDb.statTotal)}: {stats.total}
              </Badge>
              <Badge variant="outline">
                {t(WEBUI.ruleDb.statEnabled)}: {stats.enabled}
              </Badge>
              <Badge variant="outline">
                {t(WEBUI.ruleDb.stateDisabled)}: {stats.disabled}
              </Badge>
            </div>
          </div>
          <div className="flex flex-col gap-2 lg:flex-row lg:items-center">
            <div className="relative min-w-0 flex-1">
              <Search className="pointer-events-none absolute left-2.5 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-muted-foreground" />
              <Input
                value={search}
                onChange={(event) => setSearch(event.target.value)}
                placeholder={t(WEBUI.ruleDb.searchPlaceholder)}
                className="h-9 pl-8 font-mono"
              />
            </div>
            <div className="flex flex-wrap items-center gap-2">
              <Select
                value={kindFilter}
                onValueChange={(value) => setKindFilter(value as KindFilter)}
              >
                <SelectTrigger className="h-9 w-36">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="all">
                    {t(WEBUI.ruleDb.allKinds)}
                  </SelectItem>
                  <SelectItem value="domain">
                    {t(WEBUI.ruleDb.kindDomain)}
                  </SelectItem>
                  <SelectItem value="ip">{t(WEBUI.ruleDb.kindIp)}</SelectItem>
                </SelectContent>
              </Select>
              <Select value={categoryFilter} onValueChange={setCategoryFilter}>
                <SelectTrigger className="h-9 w-40">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="all">
                    {t(WEBUI.ruleDb.allCategories)}
                  </SelectItem>
                  {categoryOptions.map((option) => (
                    <SelectItem key={option.id} value={option.id}>
                      {t(WEBUI.ruleDb.categoryOption, {
                        label: option.label,
                        count: option.count,
                      })}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
              <Select
                value={stateFilter}
                onValueChange={(value) => setStateFilter(value as StateFilter)}
              >
                <SelectTrigger className="h-9 w-36">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="all">
                    {t(WEBUI.ruleDb.allStates)}
                  </SelectItem>
                  <SelectItem value="enabled">
                    {t(WEBUI.ruleDb.enabledOnly)}
                  </SelectItem>
                  <SelectItem value="disabled">
                    {t(WEBUI.ruleDb.disabledOnly)}
                  </SelectItem>
                </SelectContent>
              </Select>
            </div>
          </div>
          {(notice || error) && (
            <div
              className={
                error
                  ? "rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-xs text-destructive"
                  : "rounded-md border border-primary/30 bg-primary/10 px-3 py-2 text-xs text-primary"
              }
            >
              {error ?? notice}
            </div>
          )}
        </CardHeader>
        <CardContent className="space-y-3">
          <div className="overflow-x-auto rounded-md border">
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead className="hidden w-16 lg:table-cell">
                    {t(WEBUI.ruleDb.columnId)}
                  </TableHead>
                  <TableHead>{t(WEBUI.ruleDb.columnRule)}</TableHead>
                  <TableHead className="w-24">
                    {t(WEBUI.ruleDb.columnKind)}
                  </TableHead>
                  <TableHead className="w-24">
                    {t(WEBUI.ruleDb.columnCategory)}
                  </TableHead>
                  <TableHead className="hidden w-28 xl:table-cell">
                    {t(WEBUI.ruleDb.columnSource)}
                  </TableHead>
                  <TableHead className="w-28">
                    {t(WEBUI.ruleDb.columnState)}
                  </TableHead>
                  <TableHead className="hidden w-44 lg:table-cell">
                    {t(WEBUI.ruleDb.columnUpdated)}
                  </TableHead>
                  <TableHead className="w-28 text-right">
                    {t(WEBUI.ruleDb.columnActions)}
                  </TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {state === "loading" ? (
                  <TableRow>
                    <TableCell colSpan={8} className="py-8 text-center">
                      <span className="inline-flex items-center gap-2 text-sm text-muted-foreground">
                        <Loader2 className="h-4 w-4 animate-spin" />
                        {t(WEBUI.ruleDb.loading)}
                      </span>
                    </TableCell>
                  </TableRow>
                ) : rules.length === 0 ? (
                  <TableRow>
                    <TableCell
                      colSpan={8}
                      className="py-8 text-center text-sm text-muted-foreground"
                    >
                      {appliedSearch ||
                      kindFilter !== "all" ||
                      categoryFilter !== "all" ||
                      stateFilter !== "all"
                        ? t(WEBUI.ruleDb.noFilteredRules)
                        : t(WEBUI.ruleDb.noRules)}
                    </TableCell>
                  </TableRow>
                ) : (
                  rules.map((rule) =>
                    editing?.id === rule.id ? (
                      <TableRow key={rule.id}>
                        <TableCell className="hidden font-mono text-xs text-muted-foreground lg:table-cell">
                          {rule.id}
                        </TableCell>
                        <TableCell>
                          <Input
                            value={editing.rule}
                            onChange={(event) =>
                              setEditing({
                                ...editing,
                                rule: event.target.value,
                              })
                            }
                            className="h-8 font-mono text-sm"
                            disabled={saving}
                          />
                        </TableCell>
                        <TableCell>
                          <Select
                            value={editing.kind}
                            onValueChange={(value) =>
                              setEditing({
                                ...editing,
                                kind: value as RuleKind,
                              })
                            }
                            disabled={saving}
                          >
                            <SelectTrigger className="h-8 w-24">
                              <SelectValue />
                            </SelectTrigger>
                            <SelectContent>
                              <SelectItem value="domain">
                                {t(WEBUI.ruleDb.kindDomain)}
                              </SelectItem>
                              <SelectItem value="ip">
                                {t(WEBUI.ruleDb.kindIp)}
                              </SelectItem>
                            </SelectContent>
                          </Select>
                        </TableCell>
                        <TableCell>
                          <Select
                            value={editing.category}
                            onValueChange={(value) =>
                              setEditing({ ...editing, category: value })
                            }
                            disabled={saving}
                          >
                            <SelectTrigger className="h-8 w-24">
                              <SelectValue />
                            </SelectTrigger>
                            <SelectContent>
                              {categoryOptions.map((option) => (
                                <SelectItem key={option.id} value={option.id}>
                                  {option.label}
                                </SelectItem>
                              ))}
                            </SelectContent>
                          </Select>
                        </TableCell>
                        <TableCell className="hidden text-xs text-muted-foreground xl:table-cell">
                          {rule.source}
                        </TableCell>
                        <TableCell>
                          <Switch
                            size="sm"
                            checked={editing.enabled}
                            onCheckedChange={(checked) =>
                              setEditing({ ...editing, enabled: checked })
                            }
                            disabled={saving}
                          />
                        </TableCell>
                        <TableCell className="hidden text-xs text-muted-foreground lg:table-cell">
                          {formatTime(rule.updated_at_ms)}
                        </TableCell>
                        <TableCell className="text-right">
                          <div className="flex justify-end gap-1">
                            <Button
                              variant="ghost"
                              size="icon-sm"
                              disabled={saving}
                              onClick={() => void handleSaveEdit()}
                              aria-label={t(WEBUI.ruleDb.editSave)}
                            >
                              {saving ? (
                                <Loader2 className="h-4 w-4 animate-spin" />
                              ) : (
                                <Save className="h-4 w-4 text-primary" />
                              )}
                            </Button>
                            <Button
                              variant="ghost"
                              size="icon-sm"
                              disabled={saving}
                              onClick={() => setEditing(null)}
                              aria-label={t(WEBUI.ruleDb.editCancel)}
                            >
                              <X className="h-4 w-4" />
                            </Button>
                          </div>
                        </TableCell>
                      </TableRow>
                    ) : (
                      <TableRow key={rule.id}>
                        <TableCell className="hidden font-mono text-xs text-muted-foreground lg:table-cell">
                          {rule.id}
                        </TableCell>
                        <TableCell className="font-mono text-sm break-all">
                          {rule.rule}
                        </TableCell>
                        <TableCell>
                          <Badge
                            variant="outline"
                            className={
                              rule.kind === "ip"
                                ? "border-chart-3/40 bg-chart-3/10 text-chart-3"
                                : "border-chart-2/40 bg-chart-2/10 text-chart-2"
                            }
                          >
                            {rule.kind === "ip"
                              ? t(WEBUI.ruleDb.kindIp)
                              : t(WEBUI.ruleDb.kindDomain)}
                          </Badge>
                        </TableCell>
                        <TableCell>
                          <Badge
                            variant="outline"
                            className={
                              CATEGORY_TONES[rule.category] ??
                              CATEGORY_TONES.default
                            }
                          >
                            {categoryLabel(rule.category)}
                          </Badge>
                        </TableCell>
                        <TableCell className="hidden text-xs text-muted-foreground xl:table-cell">
                          {rule.source}
                        </TableCell>
                        <TableCell>
                          <div className="flex items-center gap-2">
                            <Switch
                              size="sm"
                              checked={rule.enabled}
                              disabled={busyId === rule.id}
                              onCheckedChange={() => void handleToggle(rule)}
                            />
                            <span className="text-xs text-muted-foreground">
                              {rule.enabled
                                ? t(WEBUI.ruleDb.stateEnabled)
                                : t(WEBUI.ruleDb.stateDisabled)}
                            </span>
                          </div>
                        </TableCell>
                        <TableCell className="hidden text-xs text-muted-foreground lg:table-cell">
                          {formatTime(rule.updated_at_ms)}
                        </TableCell>
                        <TableCell className="text-right">
                          <div className="flex justify-end gap-1">
                            <Button
                              variant="ghost"
                              size="icon-sm"
                              onClick={() =>
                                setEditing({
                                  id: rule.id,
                                  rule: rule.rule,
                                  kind: rule.kind,
                                  category: rule.category,
                                  enabled: rule.enabled,
                                })
                              }
                              aria-label={t(WEBUI.ruleDb.editTitle)}
                            >
                              <Pencil className="h-4 w-4" />
                            </Button>
                            <AlertDialog>
                              <AlertDialogTrigger asChild>
                                <Button
                                  variant="ghost"
                                  size="icon-sm"
                                  disabled={removingId === rule.id}
                                  aria-label={t(WEBUI.ruleDb.deleteButton)}
                                >
                                  {removingId === rule.id ? (
                                    <Loader2 className="h-4 w-4 animate-spin" />
                                  ) : (
                                    <Trash2 className="h-4 w-4 text-destructive" />
                                  )}
                                </Button>
                              </AlertDialogTrigger>
                              <AlertDialogContent>
                                <AlertDialogHeader>
                                  <AlertDialogMedia className="bg-destructive/10 text-destructive">
                                    <Trash2 className="h-5 w-5" />
                                  </AlertDialogMedia>
                                  <AlertDialogTitle>
                                    {t(WEBUI.ruleDb.deleteTitle)}
                                  </AlertDialogTitle>
                                  <AlertDialogDescription>
                                    {t(WEBUI.ruleDb.deleteDesc, {
                                      rule: rule.rule,
                                    })}
                                  </AlertDialogDescription>
                                </AlertDialogHeader>
                                <AlertDialogFooter>
                                  <AlertDialogCancel>
                                    {t(WEBUI.ruleDb.editCancel)}
                                  </AlertDialogCancel>
                                  <AlertDialogAction
                                    variant="destructive"
                                    onClick={(event) => {
                                      event.preventDefault();
                                      void handleDelete(rule.id);
                                    }}
                                  >
                                    {t(WEBUI.ruleDb.deleteButton)}
                                  </AlertDialogAction>
                                </AlertDialogFooter>
                              </AlertDialogContent>
                            </AlertDialog>
                          </div>
                        </TableCell>
                      </TableRow>
                    ),
                  )
                )}
              </TableBody>
            </Table>
          </div>

          <div className="flex items-center justify-between gap-2 text-xs text-muted-foreground">
            <span>
              {t(WEBUI.ruleDb.statTotal)}: {total} · {rules.length}
            </span>
            {nextCursor !== null && (
              <Button
                variant="outline"
                size="sm"
                disabled={loadingMore}
                onClick={() => void load(nextCursor)}
              >
                {loadingMore && <Loader2 className="h-4 w-4 animate-spin" />}
                {t(WEBUI.ruleDb.loadMore)}
              </Button>
            )}
          </div>
        </CardContent>
      </Card>
    </div>
  );
}

function formatTime(ms: number): string {
  if (!Number.isFinite(ms) || ms <= 0) return "-";
  try {
    return new Date(ms).toLocaleString();
  } catch {
    return "-";
  }
}
