#!/usr/bin/env bash
# Deploy the public site (calibrationscope.com) — S3 + CloudFront, atomically
# with the style-hash CSP policy.
#
# GATE (policy/HANDOFF_claude_code_gui.md): the CloudFront response-headers
# policy carries BOTH pages' <style> sha256 hashes. If the pages ship with new
# CSS but the policy still holds the old hashes, the CloudFront CSP header
# blocks the new <style> and the pages BLANK. Therefore: update the policy
# FIRST, then upload, then invalidate. This script does all three in order.
#
# Needs AWS credentials with cloudfront + s3 rights (Hermes's seat, or run
# `aws login` first). Claude Code's seat has none — by design.
set -euo pipefail

DIST_ID="E380F2PTHYDACJ"
POLICY_ID="42a28561-ee87-4c3a-8621-94187ee9e22e"
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SITE="$REPO/site"

# ── 1. Compute the CURRENT hashes from the files (single source of truth). ──
hash_of() {
  python3 - "$1" <<'PY'
import hashlib, base64, re, sys, pathlib
html = pathlib.Path(sys.argv[1]).read_text(encoding="utf-8")
inner = re.search(r"<style>(.*?)</style>", html, re.S).group(1)
print("sha256-" + base64.b64encode(hashlib.sha256(inner.encode()).digest()).decode())
PY
}
H_INDEX="$(hash_of "$SITE/index.html")"
H_LESSONS="$(hash_of "$SITE/lessons.html")"
echo "index.html   style hash: $H_INDEX"
echo "lessons.html style hash: $H_LESSONS"

# Sanity: each page's meta CSP must already carry its own hash.
grep -q "$H_INDEX" "$SITE/index.html" || { echo "FATAL: index meta CSP hash mismatch"; exit 1; }
grep -q "$H_LESSONS" "$SITE/lessons.html" || { echo "FATAL: lessons meta CSP hash mismatch"; exit 1; }

# ── 2. Update the CloudFront response-headers policy with both hashes. ─────
ETAG=$(aws cloudfront get-response-headers-policy --id "$POLICY_ID" \
  --query 'ETag' --output text)
aws cloudfront get-response-headers-policy --id "$POLICY_ID" \
  --query 'ResponseHeadersPolicy.ResponseHeadersPolicyConfig' --output json > /tmp/rhp.json
python3 - "$H_INDEX" "$H_LESSONS" <<'PY'
import json, re, sys
cfg = json.load(open("/tmp/rhp.json"))
# CLI quirk: GET returns UNCONFIGURED security headers as {} (e.g.
# XSSProtection), but UPDATE validates any present member for its required
# fields. Prune empty members or the update fails ParamValidation.
sec = cfg.get("SecurityHeadersConfig", {})
for k in [k for k, v in sec.items() if v == {}]:
    del sec[k]
csp = sec["ContentSecurityPolicy"]["ContentSecurityPolicy"]
new_style = f"style-src 'self' '{sys.argv[1]}' '{sys.argv[2]}'"
csp2 = re.sub(r"style-src [^;]*", new_style, csp)
if csp2 == csp:
    print("policy CSP already current — no update needed")
else:
    print("policy CSP style-src ->", new_style)
sec["ContentSecurityPolicy"]["ContentSecurityPolicy"] = csp2
json.dump(cfg, open("/tmp/rhp.json", "w"))
PY
aws cloudfront update-response-headers-policy --id "$POLICY_ID" \
  --if-match "$ETAG" --response-headers-policy-config file:///tmp/rhp.json > /dev/null
echo "CloudFront headers policy updated."

# ── 3. Upload site + invalidate. ───────────────────────────────────────────
# Bucket names may themselves contain dots (ours: calibrationscope.com-site),
# so strip the ".s3.<region>.amazonaws.com" suffix — never cut at the first dot.
BUCKET=$(aws cloudfront get-distribution --id "$DIST_ID" \
  --query 'Distribution.DistributionConfig.Origins.Items[0].DomainName' --output text \
  | sed -E 's/\.s3[.-][a-z0-9-]+\.amazonaws\.com$//; s/\.s3\.amazonaws\.com$//')
echo "S3 bucket: $BUCKET"
aws s3 sync "$SITE" "s3://$BUCKET" --exclude ".DS_Store" --delete
aws cloudfront create-invalidation --distribution-id "$DIST_ID" --paths "/*" > /dev/null
echo "Deployed + invalidation issued. Verify in a real browser: console must be clean."
