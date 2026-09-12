import { useState, useRef, type KeyboardEvent } from "react";
import RulesPage from "./RulesPage";
import DomainApplyPage from "./DomainApplyPage";
import { cn } from "../lib/utils";

const TABS = [
  { id: "rules", label: "规则" },
  { id: "hooks", label: "Hooks" },
] as const;

type TabId = (typeof TABS)[number]["id"];

export default function RulesHooksPage() {
  const [activeTab, setActiveTab] = useState<TabId>("rules");
  const tabRefs = useRef<Record<TabId, HTMLButtonElement | null>>({
    rules: null,
    hooks: null,
  });

  const handleKeyDown = (e: KeyboardEvent<HTMLButtonElement>, index: number) => {
    let nextIndex = -1;
    if (e.key === "ArrowRight" || e.key === "ArrowDown") {
      e.preventDefault();
      nextIndex = (index + 1) % TABS.length;
    } else if (e.key === "ArrowLeft" || e.key === "ArrowUp") {
      e.preventDefault();
      nextIndex = (index - 1 + TABS.length) % TABS.length;
    } else if (e.key === "Home") {
      e.preventDefault();
      nextIndex = 0;
    } else if (e.key === "End") {
      e.preventDefault();
      nextIndex = TABS.length - 1;
    }

    if (nextIndex >= 0) {
      const nextTab = TABS[nextIndex].id;
      setActiveTab(nextTab);
      tabRefs.current[nextTab]?.focus();
    }
  };

  return (
    <div className="space-y-4">
      <div role="tablist" aria-label="规则与 Hooks" className="flex border-b border-slate-200">
        {TABS.map((tab, idx) => {
          const isSelected = activeTab === tab.id;
          return (
            <button
              key={tab.id}
              ref={(el) => {
                tabRefs.current[tab.id] = el;
              }}
              type="button"
              role="tab"
              id={`tab-${tab.id}`}
              aria-controls={`tabpanel-${tab.id}`}
              aria-selected={isSelected}
              tabIndex={isSelected ? 0 : -1}
              onClick={() => setActiveTab(tab.id)}
              onKeyDown={(e) => handleKeyDown(e, idx)}
              className={cn(
                "border-b-2 px-4 py-2 text-sm font-medium transition-colors focus:outline-none focus:ring-2 focus:ring-indigo-500 focus:ring-offset-2",
                isSelected
                  ? "border-indigo-600 font-semibold text-indigo-600"
                  : "border-transparent text-slate-500 hover:border-slate-300 hover:text-slate-700",
              )}
            >
              {tab.label}
            </button>
          );
        })}
      </div>

      <div
        role="tabpanel"
        id={`tabpanel-${activeTab}`}
        aria-labelledby={`tab-${activeTab}`}
        tabIndex={0}
        className="focus:outline-none"
      >
        {activeTab === "rules" ? (
          <RulesPage />
        ) : (
          <DomainApplyPage
            domain="hooks"
            title="Hooks"
            description="助手钩子定义的写入与同步状态。"
          />
        )}
      </div>
    </div>
  );
}
