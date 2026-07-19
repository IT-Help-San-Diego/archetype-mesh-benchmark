# Local-Access Strategy

How users reach the Calibration Scope dashboard — and the security/honesty
principles that govern it. This is a **foundation document**: it records a
deliberate design decision, verified against real DNS behavior, so future
work builds on facts rather than assumptions.

## The Verizon pattern (what we learned from)

Verizon ships home routers that respond to a friendly domain
(`mynetworksettings.com`) instead of a bare IP. That domain publishes a public
DNS record:

```
mynetworksettings.com.  IN  A  127.0.0.1
```

Two halves make the trick work:

1. **Public half (any registrant can do this):** the A record points at
   `127.0.0.1` (loopback). When you type the domain in a browser, DNS returns
   your own loopback. No router cooperation needed.
2. **ISP half (only the ISP can do this):** the router runs a local DNS
   override that maps the same domain to `192.168.1.1`, so customers actually
   land on the router admin page. This requires owning the CPE firmware and
   (for their customers) the resolving DNS.

We only need the **public half**. Our dashboard and the user's browser run on
the same machine, so `127.0.0.1` is exactly where we want them.

## Our three-tier local-access model

When we have a domain (`calibrationscope.com` — registered 2026-07-19;
supersedes the earlier `archetypemesh.com`/`carrierscope.com` plans), offer:

| Tier | Address | Needs | Use case |
|------|---------|-------|----------|
| **Friendly subdomain** | `local.calibrationscope.com A 127.0.0.1` | a registered domain | Verizon-style friendly name; works from any network |
| **mDNS / .local** | `calibrationscope.local` (Bonjour/Avahi) | service advertises on LAN | zero-registrar LAN access when dashboard runs on a box others reach |
| **Raw IP** | `127.0.0.1:8768` (same machine) / LAN IP (shared) | nothing | always-works fallback; no DNS dependency |

A subdomain under a domain we own costs nothing extra and behaves identically
to a new domain. Prefer `local.archetypemesh.com` over registering a separate
name.

## Is publishing a loopback record "evil"? No.

`localtest.me` and `lvh.me` are independent public domains (run by ordinary
developers, not giants) that resolve to `127.0.0.1` and are trusted industry-wide
dev tooling. Pointing a domain at loopback is a benign, decades-old convention
— not a privileged capability and not an attack. The malicious cousin is
**DNS rebinding** (rapidly swapping `127.0.0.1` ↔ a real IP to attack localhost
services); we are explicitly NOT doing that.

## Verified caveats (test before relying on the friendly domain)

Empirically confirmed 2026-07-19 via `dig`:

- **DoH filtering inconsistency:** Cloudflare (`1.1.1.1`) returns
  `127.0.0.1`; Google (`8.8.8.8`) **drops the loopback answer** (times out).
  A user on Google DNS will NOT resolve the friendly domain.
- **DNS-rebinding protection** in some routers/browsers may block loopback
  answers for external-origin names. Test the subdomain specifically.
- **Same-machine only:** `127.0.0.1` works only when browser + dashboard are
  on the SAME machine. A dashboard on a LAN server needs the LAN IP or
  `.local` (mDNS), not the loopback domain.

## Security-honesty principle (mandatory)

The dashboard binds `127.0.0.1` by **default** — meaning Wi-Fi neighbors
CANNOT reach it. This is the safe default and must not change without explicit
user action.

If a user opts into LAN exposure (binds `0.0.0.0` to share with their phone
or another machine), the UI MUST show a clear warning banner:

> "This dashboard is reachable by anyone on your Wi-Fi network."

This is the same honesty principle as the RUN ACTIVE banner: the user is never
surprised about who can see their data. Never ship a localhost admin tool with
a default that exposes it to the LAN.

## Implementation checklist (when the domain exists)

- [ ] Register `archetypemesh.com` (or final name).
- [ ] Add `local.archetypemesh.com A 127.0.0.1` at the registrar.
- [ ] Document the Google-DNS / rebinding caveats in the user-facing setup
      guide; instruct fallback to `127.0.0.1:8768` or `arctypemesh.local`.
- [ ] Keep loopback bind as the default; gate LAN exposure behind an explicit
      opt-in + warning banner.
- [ ] (Optional) Add mDNS advertisement for LAN-box deployments.

## Status

- Dashboard already works at `127.0.0.1:8768` and LAN IP today.
- Friendly-domain + mDNS are a **later polish layer**, not a re-architecture.
- No domain registered yet (budget). Design is documented so the add is
  mechanical when funding allows.
