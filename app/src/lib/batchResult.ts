import type { BatchApplyRow } from "./useBatchApply";

export interface BatchResult {
  id: string;
  command: string;
  label: string;
  running: boolean;
  rows: { row: BatchApplyRow; status: "pending" | "ok" | "failed"; message?: string }[];
}

export function batchCounts(batch: BatchResult) {
  return {
    succeeded: batch.rows.filter((item) => item.status === "ok").length,
    failed: batch.rows.filter((item) => item.status === "failed").length,
    pending: batch.rows.filter((item) => item.status === "pending").length,
  };
}
