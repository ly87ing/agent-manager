import { describe, expect, it, vi } from "vitest";
import { subscribeImportLinks, type ImportLinkClient } from "../lib/shareLink";

const url = "agentmanager://import?data=example";
function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}
async function settle() { await Promise.resolve(); await Promise.resolve(); }
function fixture() {
  const ready = deferred<() => void>();
  const current = deferred<unknown>();
  let event!: (value: unknown) => void;
  const stop = vi.fn();
  const onLink = vi.fn();
  const client: ImportLinkClient = {
    listen: vi.fn((callback) => { event = callback; return ready.promise; }),
    current: vi.fn(() => current.promise),
  };
  const dispose = subscribeImportLinks(onLink, client);
  return { ready, current, client, stop, onLink, dispose, event: (value: unknown) => event(value) };
}

describe("desktop share-link subscription", () => {
  it("reads a cold-start link only after the runtime listener is ready", async () => {
    const f = fixture();
    expect(f.client.current).not.toHaveBeenCalled();
    f.ready.resolve(f.stop);
    await settle();
    expect(f.client.current).toHaveBeenCalledOnce();
    f.current.resolve([{ id: 1, url }]);
    await settle();
    expect(f.onLink).toHaveBeenCalledExactlyOnceWith(url);
    f.dispose();
    expect(f.stop).toHaveBeenCalledOnce();
  });

  it("handles warm openings, including later clicks on the identical URL", async () => {
    const f = fixture();
    f.ready.resolve(f.stop);
    f.current.resolve([]);
    await settle();
    f.event({ id: 1, url });
    f.event({ id: 2, url });
    expect(f.onLink.mock.calls).toEqual([[url], [url]]);
    f.dispose();
  });

  it.each(["event-first", "snapshot-first"])("deduplicates the startup race in either order: %s", async (order) => {
    const f = fixture();
    f.ready.resolve(f.stop);
    await settle();
    if (order === "event-first") f.event({ id: 1, url });
    f.current.resolve([{ id: 1, url }]);
    await settle();
    if (order === "snapshot-first") f.event({ id: 1, url });
    expect(f.onLink).toHaveBeenCalledExactlyOnceWith(url);
    f.event({ id: 2, url });
    expect(f.onLink).toHaveBeenCalledTimes(2);
    f.dispose();
  });

  it("does not let an older initial snapshot replace a newer runtime opening", async () => {
    const f = fixture();
    f.ready.resolve(f.stop);
    await settle();
    f.event({ id: 2, url: url + "-new" });
    f.current.resolve([{ id: 1, url }]);
    await settle();
    f.event({ id: 1, url });
    expect(f.onLink.mock.calls).toEqual([[url + "-new"]]);
    f.event({ id: 3, url });
    expect(f.onLink.mock.calls).toEqual([[url + "-new"], [url]]);
    f.dispose();
  });

  it("unmounts before listener registration without consuming initial links", async () => {
    const f = fixture();
    f.dispose();
    f.ready.resolve(f.stop);
    await settle();
    f.event({ id: 1, url });
    expect(f.stop).toHaveBeenCalledOnce();
    expect(f.client.current).not.toHaveBeenCalled();
    expect(f.onLink).not.toHaveBeenCalled();
  });

  it("unmounts during the initial read without a late callback or duplicate cleanup", async () => {
    const f = fixture();
    f.ready.resolve(f.stop);
    await settle();
    f.dispose();
    f.current.resolve([{ id: 1, url }]);
    f.event({ id: 2, url });
    await settle();
    f.dispose();
    expect(f.stop).toHaveBeenCalledOnce();
    expect(f.onLink).not.toHaveBeenCalled();
  });

  it("retains runtime events if the initial read fails and rejects invalid payloads", async () => {
    const f = fixture();
    f.ready.resolve(f.stop);
    await settle();
    f.current.reject(new Error("read failed"));
    await settle();
    for (const bad of [url, null, { id: 0, url }, { id: 1.5, url }, { id: 1, url: "https://example.com" }, { id: 1, url: url + "a".repeat(64 * 1024) }]) f.event(bad);
    expect(f.onLink).not.toHaveBeenCalled();
    f.event({ id: 1, url });
    expect(f.onLink).toHaveBeenCalledExactlyOnceWith(url);
    expect(f.stop).not.toHaveBeenCalled();
    f.dispose();
  });
});
