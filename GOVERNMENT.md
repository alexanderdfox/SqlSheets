# SQLSheet — Government / Public-Sector Positioning

This folder contains an updated `index.html` and `README.md` reframed for government and regulated use.

## What changed (high level)

### index.html (UI)
- Title and meta description emphasize secure, offline, zero-egress operation for public-sector work.
- Logo tagline: “Offline SQLite · Zero data egress”.
- Welcome screen rewritten for professional / regulated audiences (FOIA, inventory, budgets, field work).
- Security posture callout on the empty state (CSP, SRI, scripts off by default, no analytics, air-gap suitable).
- Game of Life demo de-emphasized as optional.
- Skip-to-content link and basic ARIA roles for accessibility.
- Button tooltips clarify that data stays local.
- Security model comment in `<head>` expanded for auditors.

### README.md
- Lead with government-relevant benefits table (data sovereignty, zero telemetry, air-gap, auditability).
- Explicit disclaimer: **not** FedRAMP / FISMA / CMMC authorized.
- Public-sector use cases (FOIA prep, inventory, budgets, field, training).
- Security model summary suitable for risk assessments.
- Accessibility notes and self-hosting guidance for internal networks.
- Clear statement that formal ATO remains the deploying organization’s responsibility.

## How to deploy

1. Replace the repository’s `index.html` and `README.md` with the versions in this folder (or open a PR).
2. For government networks, prefer **self-hosting** the static files rather than relying on GitHub Pages.
3. For air-gapped use: load once on a connected machine (or vendor media), then transfer via approved channels.

## What this does *not* claim

- FedRAMP, FISMA, CMMC, or any formal authorization.
- Multi-user concurrent editing or enterprise identity integration.
- WCAG 2.2 AA certification (further testing recommended for procurement).

The product remains a transparent, local-first open-source workbench. The updates make that posture explicit and professional for public-sector evaluators.
