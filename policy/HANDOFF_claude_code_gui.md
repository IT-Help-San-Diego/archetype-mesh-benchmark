# HANDOFF to Claude Code — GUI/UX lane (updated 2026-07-22, post-Safari-fix)

Your lane is frontend/UX across BOTH surfaces: the local dashboard
(127.0.0.1:8768) and the public site (calibrationscope.com). State is current
as of commit eef6d20+ and the Safari white-page fix (upgrade-insecure-requests
removed from the LOCAL CSP only — see "Hard-won lessons" below before you
touch any CSP).

Read `policy/EPISTEMIC_LOG_POLICY.md` first — it governs how you log any
re-runs or data changes you make.

## Gate rules (hard, do not skip)

- **Zero executable JS on the public site.** script-src 'none'. The only
  allowed exception is application/ld+json data blocks.
- **style-src hash rule (public site).** The CloudFront CSP carries BOTH
  pages' style hashes ('self' + sha256 of each page's <style> block).
  Recompute on EVERY CSS change or the page blanks. Policy id
  42a28561-ee87-4c3a-8621-94187ee9e22e.
- **CSP is different per surface — and on the LOCAL surface, per CONNECTION**
  (updated 2026-07-22, local HTTPS shipped). Public site = full hardening incl.
  upgrade-insecure-requests (correct — real TLS). Local dashboard now speaks
  BOTH protocols on one port (first-byte peek → rustls or plain HTTP):
  upgrade-insecure-requests is emitted ONLY on TLS connections
  (security.rs::csp takes an `https` flag from the per-connection ConnScheme
  extension). On a plain-HTTP connection the directive would command Safari to
  refetch assets over TLS the client may not trust — the white-page bug. Do
  NOT make it unconditional in either direction, and do NOT copy a CSP
  between surfaces blindly.
- **Verify in the live browser, not by curl.** Firefox MCP
  (mcp__firefox_devtools__*) is the instrument: navigate, evaluate_script for
  computed sizes + sheetCount, list_console_messages. The
  browser-console-preflight skill is mandatory before any HTML/CSS edit.
- **No spinners.** Every loading state shows real data or nothing.
- **Lighthouse ≥ 91 perf / 100 a11y / 100 bp / 100 seo** (desktop preset)
  on the public site; 90/98/100/91 on the dashboard.
- **Accessibility is the default.** Readable/High-contrast is ON first visit.
- Commit + push immediately after each verified change (cross-agent record
  duty: whichever agent does the work updates DECISIONS.md itself).

## Hard-won lessons (read before you debug)

1. **The Safari white page was NOT bfcache, caching, nonce mismatch, or a JS
   bug.** It was `upgrade-insecure-requests` in the LOCAL dashboard's CSP.
   Safari honors it: it upgraded its own subresource URLs
   (`/assets/app.min.css` → `https://127.0.0.1:8768/...`) and died because
   nothing on 8768 speaks TLS. Every asset failed with "network connection
   lost", `showPage` was undefined, white tool. Firefox's loopback carve-out
   hid it. **Fixed by removing that directive from the LOCAL CSP only**
   (commit eef6d20 + the follow-up). The public site keeps it — it's correct
   there.
2. **Verify against the LIVE resolver/source, never from memory.** Three
   separate "impossible" claims this session (cost figures, Cognitive Atlas
   counts, nonce mismatches) were all wrong in MY verification method, not
   the instrument. When something "can't be true," re-measure first.
3. **The nonce stamping is now single-source.** The middleware stamps BOTH
   the CSP header AND the HTML body with the same per-request nonce. Don't
   split them again — the handler's stamp is a no-op after the middleware
   pass by design.
4. **Assets are inlined/self-hosted deliberately.** The site has no external
   subresources; every image/script/style is same-origin. Don't introduce a
   CDN or external font.

## Open items (pick in order)

1. ~~Local HTTPS~~ **DONE (Claude Code, 2026-07-22).** Dual-protocol on ONE
   port (8768): first-byte peek routes TLS → rustls, everything else → plain
   HTTP — so no existing http consumer (curl, Python client, Hermes scripts,
   launchd checks) broke, and trusting the CA is an opt-in upgrade, never a
   prerequisite. Self-provisioned CA + leaf via rcgen (src/local_tls.rs):
   `~/.calibration-scope/ca/`, SANs local.calibrationscope.com + localhost +
   127.0.0.1 + ::1, leaf 820-day validity + serverAuth EKU (Apple's 825-day
   rule honored), keys 0600. upgrade-insecure-requests restored on TLS
   connections only (per-connection ConnScheme). Trust: double-click
   ca.cert.pem or `scripts/trust-local-ca.sh`. Crypto: rustls + ring (audited,
   zero extra toolchain); FIPS 140-3 available as opt-in `--features fips`
   (AWS-LC FIPS) — decided ring-default because most scientists don't need
   FIPS and it costs cmake/go build friction. Verified live: chain+hostname
   validation (ssl_verify_result=0), IP SAN, SSE over TLS, CSP split, 36 unit
   tests, clippy 0.
2. **Site polish — owl+brain graphic pass.** BUILT (Claude Code, 2026-07-23,
   commit 7ed1af9) — DEPLOY PENDING (`scripts/deploy-site.sh` from a
   credentialed seat; this seat has no AWS creds by design):
   - Canonical LOCAL⇄WEB portal pill on ALL THREE surfaces (site home,
     lessons, local instrument): fixed top:10px/left:14px, identical string
     + size (87×25) — flipping never moves the control under the cursor
     (Carey's no-mouse-jerk rule). Current surface gold; whole pill links.
   - The spec-stream on the site: 8 real battery schemas (⊢/⊬ ground truth)
     rising past the brain — inline-SVG text + CSS-only @keyframes, zero JS,
     stagger via classes (style attrs are CSP-blocked), reduced-motion →
     static faint formulas. viewBox matched to the brain art's real
     landscape aspect (square viewBox letterboxed the stream — caught live).
   - Both pages' style hashes recomputed + stamped in meta CSPs; verified
     in-browser: CLEAN console, 8 animations, pill pinned across scroll.
   - Remaining for this item: run deploy-site.sh (Hermes/Carey), then a
     final visual pass on the LIVE site; optionally richer brain art later.
3. **Lessons page polish.** Four comics render inline; design pass on the
   lesson cards, status badges, seal lines. Do NOT change the lesson .md
   files or comic SVGs (sealed — hash-verified).
4. **Human-calibration UI polish (dashboard).** Backend is DONE (5
   endpoints, E2E verified). Frontend is functional but basic (4-step flow
   at page-human-cal). Add: per-question timing display, a carrier-variance
   bar chart at results, and a human-vs-model comparison panel (the
   signal_carrier endpoint already returns both subjects in the same shape).
5. **Architecture diagram.** docs/architecture.excalidraw is stale — add
   the Focused shell, NeuroVault proxy, signal-carrier view, spec-decode
   panel, human-calibration page, /api/runs/complete endpoint, MCP server.
6. **OWL N/C coverage expansion.** LOGIC-05/07/08/09/10 still have no N/C
   siblings. Template = migration 047/048 pattern (same formal_spec, new
   surface text for N; transform + named owl_flaw for C; resolve roots by
   NAME, never raw id). Oracle: scripts/verify_logic_ground_truth.py
   --check-owl-families.
7. **MCP server e2e test.** Connect a real bot to POST /mcp, discover the
   11 tools, call run_benchmark + get_run. Untested end-to-end by a client.

## What's DONE (don't redo)

- calibrationscope.com: ACM, S3 (OAC, private), CloudFront E380F2PTHYDACJ,
  Route53 aliases, headers policy, DNSSEC, mail lockdown. Homepage +
  lessons.html live, verified in-browser.
- LOCAL⇄WEB flipper links directly to http://127.0.0.1:8768 (loopback,
  no port-forward trick — local.calibrationscope.com DNS is the alias).
- Kokoro TTS watchdog, EC2 idle-shutdown (certified), evidence eviction,
  CI green (all 4 jobs), epistemic record reconciled (§10.13), the Safari
  white-page root cause found and fixed.
- Carrier Color §10.8 is sealed; the paired-run harness is in analysis/.

## Lane boundary

- Your lane: frontend/UX/design on both surfaces.
- Claude Science's lane: Carrier Color §10.8 rewrite (when the paired CSV
  lands), seL4 root-task build (Rust compile errors → its lane, don't force
  it green), l4v proof run, EC2 ops.
- Hermes's lane: dashboard backend/executor, CI, cost, fleet, CSP/security
  middleware.
