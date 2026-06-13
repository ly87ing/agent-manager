# Writing Guidelines (reader-facing documents)

Apply when writing documents meant to be read: plans, reports, proposals, specs, checklists, HTML decks. Not code comments (see `style.md`); for HTML rendering see `frontend.md`.

- **Target the reader.** Fix who reads it and what decision or action they need; set depth and terms to them.
- **State scope and key point up front.**
- **Lead every section, paragraph, and item with its takeaway.**
- **Keep the source's framework when condensing.** Reword a layer; never delete it.
- **Keep peer items parallel.** Same fields in the same order across all items.
- **Cut words.** Delete every word that doesn't change the meaning. Active voice, specific verbs, one idea per sentence, no ambiguous pronouns.
- **One fact, one place.** Don't restate the same detail or explanation in multiple places; cross-reference instead. A deliberate lead or summary of the key point is fine — that's not the redundancy to cut.
- **Format for scanning.** Turn long sentences into lists; start numbered items with imperative verbs.
- **Prefer a diagram for a process.** When a flow or sequence reads clearer as a diagram than as prose or a table, show it once as a diagram — don't also narrate it step by step. Add a text fallback if the medium may not render it.
- **Use plain language.** Define each term on first use and use it consistently. No marketing tone, no filler.
- **Don't narrate the document itself.** Write the content, not remarks about the document — what version or section it is, or what it will cover. The reader already knows what they opened; state the point directly.
- **Be concrete.** Replace vague quantifiers with specifics: counts, thresholds, names. Back key claims with data or a concrete example.
- **Don't invent to look complete.** If a number, date, owner, or status isn't real yet, mark it TBD or cut it — never manufacture SLAs, timelines, metrics, or states to fill out a template. Concrete means *real*, not merely plausible.
- **Match scope to the real need.** Don't over-build process the situation doesn't warrant, and never present a future or aspirational state as current — mark it as proposed.
- **Give plans and action items a specific time and explicit done-criteria.** Use an absolute date when a deadline is committed; otherwise a concrete week or phase — never "now/later".
- **Stay faithful to the source.** Use its wording and figures; don't invent names; verify uncertain terms or data first.
- **Read top to bottom.** Don't hide content behind tabs or collapsibles; use a simple table only for multi-dimension comparison, never a complex grid.
- **For interactive / HTML deliverables, set accessible, self-contained defaults up front.** Large readable type, single-file/offline, click→centered modal (not a scroll-away panel), one visual per flow, full clickable hit areas, and verify the rendered result before done — see `frontend.md`. Decide these before building to avoid redo rounds on font size, redundant visuals, or dead clicks.
- **Revise before done.** Reread as the target reader: cut redundancy, verify facts, confirm each section leads with its takeaway.
