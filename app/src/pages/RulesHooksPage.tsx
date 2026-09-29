import { useState, useRef, type KeyboardEvent } from "react";
import RulesPage from "./RulesPage";
import HooksPage from "./HooksPage";
import { cn } from "../lib/utils";

const TABS = [
  { id: "rules", label: "规则" },
  { id: "hooks", label: "自动命令（Hooks）" },
] as const;

type TabId = (typeof TABS)[number]["id"];

export default function RulesHooksPage({ onNavigate }: { onNavigate?: (page: string) => void } = {}) {
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
      <div role="tablist" aria-label="规则与自动命令" className="flex border-b">
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
                "border-b-2 px-4 py-2 text-sm font-medium transition-colors focus:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2",
                isSelected
                  ? "border-primary font-semibold text-primary"
                  : "border-transparent text-muted-foreground hover:border-input hover:text-foreground",
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
        {activeTab === "rules" ? <RulesPage onOpenSkills={onNavigate ? () => onNavigate("skills") : undefined} /> : <HooksPage />}
      </div>
    </div>
  );
}
