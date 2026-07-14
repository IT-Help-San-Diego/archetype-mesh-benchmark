# Dashboard rendering failure — plan to repair

## Observed state (verified)

- Backend healthy: HTTP 200 on `http://127.0.0.1:8768/`
- Docker healthy: `archetype-postgres` bound to `127.0.0.1:5432`
- HTML structure parser reports 0 mismatches, all tags balanced
- 4 inline `<script>` blocks + 2 defer-loaded vendor scripts
- Console warning: `[selfheal] missing=selectModel,showPage,loadModels,loadLmStudioPage,apiFetch`
- Browser accessibility snapshot: raw JS template-literal fragments visible as `StaticText` on the page
- No CSS load errors confirmed, no console `SyntaxError`/`ReferenceError` exceptions captured

## Most likely failure mode

JS execution is halting early, before the functions that `selfheal` expects are registered. The visible raw `' + value + (unit || '') + '` and `const assessment = ...` fragments are template-literal fragments that were never parsed as code — they’re sitting in the DOM as text.

Because the browser *can* render the HTML but can’t complete JS initialization, the most probable cause is one of:

1. **Syntax error** in one of the large script blocks (the dossier modal render function has many nested template literals and backticks).
2. **Scope / global-leak protection** redefined the app container and silently swallowed the main init.
3. **Script ordering / defer conflict** between the inline init scripts and the KaTeX defer scripts.

## Fix plan (ordered by likelihood/speed)

### 1) Isolate the failing script block (5 min)
- Open DevTools → Sources → enable “Pause on exceptions”
- Reload the page
- Read the exact line throwing the error
- If it’s in Script Block 3 (the large 7KB block with template literals), that confirms diagnosis

### 2) Validate Script Block 3 template literals (10 min)
- Script Block 3 has 18 backticks (even count = structurally balanced *if* each pair is on the same execution path)
- Risk: multiline template literals with escaped backticks, or a `' + ... + '`  fragment accidentally lifted out of a template literal by a bad edit
- Action: extract Script Block 3 to a standalone `.js` file, run it through Node syntax check (`node --check`)

### 3) Check selfheal / anti-global-leak wrapper (5 min)
- Look for any IIFE or `(() => { ... })();` wrapping that redefines the app namespace
- Verify `window.selectModel`, `window.showPage`, etc. are being assigned, not just declared with `const`/`let` at module scope

### 4) Revert recent changes to isolate (10 min)
- The last non-trivial change was inserting the two-layer brain HTML and the dossier modal render refactor
- If reverting just the modal render section restores the page, that’s the culprit
- Use `git revert -n <commit>` and test, then re-apply piece by piece

## Immediate mitigation while investigating

If we can’t fix it today:
- Serve the old `assets/dashboard.html` from git history one commit before the modal render refactor
- Keep the new brain, SSE firing, and formula stream code on a feature branch
- Merge only after the page is provably fixed

## Cannot-do list without Nous Portal credits or local vision model

- Visually verify brain glow / formula stream / layout (vision tool returns 404 from unpaid Nous balance)
- Do NOT assume a local vision model is free to load — LM Studio memory pressure from benchmark executor + Postgres leaves limited headroom

---

## RESOLVED 2026-07-14

**Root cause (confirmed via git pickaxe + line forensics):** commit `47290ce`
("Replace inline brain SVG") deleted 2,650 lines too many. Its replacement
end-anchor matched the string `'</svg>' +` INSIDE the dossier-modal JS
(around old line 3957) instead of the real `</svg>` at line 1438. Everything
between — the rest of the page HTML, the `<script>` open tag, and the first
half of the main JS (showPage, selectModel, loadModels, apiFetch,
loadLmStudioPage) — was destroyed. File shrank 4,497 → 1,846 lines.

**Symptoms explained:** orphaned tail of the dossier renderer sat outside any
script tag (raw JS as page text); selfheal reported the five functions
missing; stray `</script>` at 1832 had lost its opener.

**Fix:** reconstructed from last-good `877ea16` + surgically re-applied the
two intended changes: (1) two-layer hero brain block (PNG + 6 ellipse
overlays), (2) new opacity-based region CSS with hover + pulse.

**Verification:** all 3 inline script blocks pass `node --check`; script tags
balanced 5/5; zero JS-leak markers in visible text; every onclick handler has
a definition; all five selfheal functions present exactly once; PNG serves
200. Note: headless-Chrome --dump-dom hangs on this page by design (SSE keeps
the connection open) — use static checks + live browser instead.
