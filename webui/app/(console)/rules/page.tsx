"use client";

import { AppHeader } from "@/components/shell/app-header";
import { RuleDbView } from "@/components/rules/rule-db-view";
import { WEBUI } from "@/lib/i18n";
import { useI18n } from "@/lib/i18n/provider";

export default function RulesPage() {
  const { t } = useI18n();

  return (
    <div className="flex h-full min-h-0 flex-col overflow-hidden">
      <AppHeader title={t(WEBUI.shell.rules)} />
      <div className="min-h-0 flex-1 overflow-y-auto p-4 md:p-6">
        <RuleDbView />
      </div>
    </div>
  );
}
