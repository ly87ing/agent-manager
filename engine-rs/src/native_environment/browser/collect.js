"use strict";
const status = document.getElementById("status");
const cancel = document.getElementById("cancel");
let cancelled = false;
cancel.onclick = async () => {
  cancelled = true;
  try { await fetch("cancel", { method: "POST", headers: { "Content-Type": "application/json" }, body: "{}" }); } catch {}
  cancel.disabled = true; status.textContent = "Check cancelled. Return to Agent Manager.";
};
(async () => {
  try {
    const low = navigator.userAgentData;
    let high = null;
    if (low) {
      try { high = await Promise.race([low.getHighEntropyValues(["architecture", "bitness", "platformVersion", "fullVersionList"]), new Promise(resolve => setTimeout(() => resolve(null), 2000))]); } catch {}
    }
    const hints = low ? { brands: low.brands, mobile: low.mobile, platform: low.platform,
      architecture: high?.architecture ?? null, bitness: high?.bitness ?? null,
      platform_version: high?.platformVersion ?? null, full_version_list: high?.fullVersionList ?? null } : null;
    const intl = Intl.DateTimeFormat().resolvedOptions();
    const sample = { origin: location.origin, timezone: intl.timeZone ?? null, languages: Array.from(navigator.languages),
      intl_locale: intl.locale ?? null, user_agent: navigator.userAgent, platform: navigator.platform,
      client_hints: hints, screen: { width: screen.width, height: screen.height, available_width: screen.availWidth,
        available_height: screen.availHeight, inner_width: innerWidth, inner_height: innerHeight,
        outer_width: outerWidth, outer_height: outerHeight, device_pixel_ratio: devicePixelRatio,
        visual_scale: visualViewport?.scale ?? null, color_depth: screen.colorDepth } };
    if (cancelled) return;
    const headers = await fetch("headers", { cache: "no-store" });
    if (!headers.ok) throw new Error("Request headers could not be observed.");
    if (cancelled) return;
    const response = await fetch("result", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(sample) });
    if (!response.ok) throw new Error("The observation was refused or expired.");
    const report = await response.json();
    if (cancelled) return;
    cancel.disabled = true;
    status.textContent = "Observation received. Return to Agent Manager for system and browser results. Only the listed criteria were compared.";
    for (const check of report.checks) {
      const li = document.createElement("li");
      li.textContent = `${check.id}: ${check.state} — ${check.observed ?? "unknown"}${check.expected === null ? "" : ` (expected ${check.expected})`}`;
      document.getElementById("checks").append(li);
    }
    const details = document.getElementById("details"); details.hidden = false; details.textContent = JSON.stringify(report, null, 2);
    setTimeout(() => {
      status.textContent = "These observations have expired. Run a new check in Agent Manager. Listed values are historical.";
    }, Math.max(0, Date.parse(report.expires_at) - Date.now()));
  } catch (error) {
    if (!cancelled) status.textContent = `${error.message} Return to Agent Manager and try a new check.`;
  }
})();
