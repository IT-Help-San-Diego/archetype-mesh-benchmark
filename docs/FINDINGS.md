# Calibration Scope — Published Findings

**Silicon & carbon under one instrument.** Local + cloud models verified against
identical stimuli, sealed with SHA-3 provenance. This is the living record of
what we've measured — not marketing, not vibes. Every number is a real run.

_Last updated: 2026-07-22._

---

## 1. Carrier Color — a model's verdict tracks the CARRIER, not the signal

**The question:** when you give a model the SAME logical content (modus
ponens/tollens, converse/inverse invalid, universal-vs-existential) wrapped in a
DIFFERENT carrier (English prose / formal Lean symbols / poetic compression /
social flattery), does the verdict change? No answer-leakage: every scaffold is
domain-general, never a test-specific formula.

**The experiment (gemma-4-e2b, 2B, 99% baseline, 102-trial clean runs):**

| Carrier | Score | vs Baseline |
|---|---|---|
| none (baseline) | **99.0%** (101/102) | — |
| Haiku (poetic compression) | **97.1%** (99/102) | −1.9 |
| English prose ("carefully track implication…") | **94.1%** (96/102) | −4.9 |
| Lean formula (`P → Q, P ⊢ Q … ⊬`) | **91.2%** (93/102) | −7.8 |
| Bribe ("you're brilliant, I'd love it, make the user happy") | **91.2%** (93/102) | −7.8 |

**Findings:**
- Every carrier drags the small near-ceiling model. Haiku (poetic compression)
  is the gentlest noise; Lean (formal symbol) and bribe (flattery) are the
  heaviest. Both inverse hypotheses ("Lean is clean", "flattery lifts") were
  **falsified**.
- The model was trained on human text, so artfully-compressed human language
  (haiku) is its native register; formal notation and social flattery are both
  alien/heavy. **The carrier (human prose) became the signal; the signal
  (formal logic) became the noise.**

## 2. Carrier-immunity threshold — big models shrug off ALL carrier noise

Replication on stronger models (same 29-test LOGIC cluster, modular `test_ids`):

| Model | Baseline | English | Lean | Haiku | Bribe | Verdict |
|---|---|---|---|---|---|---|
| gemma-4-e2b (2B) | 99.0% | 94.1% | 91.2% | 97.1% | 91.2% | **carrier-SENSITIVE** |
| nemotron-3-nano-omni (30B) | 100% | 100% | 100% | 100% | 100% | **carrier-IMMUNE** |
| anthropic/claude-fable-5 (cloud) | 100% | 100% | 100% | 100% | 100% | **carrier-IMMUNE** |

**Finding:** carrier-immunity tracks **capability/headroom**, not substrate
(local vs cloud). Small models get "neutered" by carrier noise — the carrier
crowds out their limited reasoning headroom. Big models have surplus headroom —
they absorb the noise AND keep the logic. **Below a capability threshold, a
model is carrier-sensitive; above it, carrier-immune.** Confirmed on BOTH local
(nemotron) and cloud (Fable 5).

## 3. Verified leaderboard (local models, reasoning axis, clean post-fix runs)

Top performers (pass rate on the reasoning battery, clean infrastructure):

| Model | Score | Notes |
|---|---|---|
| nvidia/nemotron-3-nano-omni | **100%** (540/540) | carrier-immune |
| openai/gpt-oss-120b | **100%** (102/102) | |
| google/gemma-4-31b | 97.6% | first verified 4/4 local |
| nvidia/nemotron-3-nano-4b | 96.9% | |
| openai/gpt-oss-20b | 96.1% | |
| google/gemma-4-e2b | 93.6% | the Goldilocks 2B (vision ✓) |
| hermes-3-llama-3.1-8b | 89% | smallest genuinely-usable non-vision |

**Goldilocks floor chain** (the lightest model that runs a real test):
- <1.5B breaks (qwen2.5-0.5b 46%, qwen2.5-0.5b-mlx 41%, llama-3.2-1b 47%).
- 1.5B barely works (qwen2.5-1.5b 65%).
- **2B (gemma-4-e2b) is genuinely usable at 99%** — the Goldilocks zone.

## 4. The "free bot" honesty check — Fountain / Trickle / Mirage

A model marked "free" might be a gift horse. The fountain probe measures the
**rate posture** (does it actually flow, or is it rate-limited, or does the
"free" claim fail outright?):

| Posture | Meaning | Models |
|---|---|---|
| **FOUNTAIN** | Flows freely | 12 (sonnet-5, step-3.7-flash, gemini-2.5-flash, deepseek, gpt-5.4, grok-4.20, llama-4-scout, glm-5.2, hermes-4-70b, …) |
| **THROTTLED** | Free but rate-limited | 1 (`stepfun/step-3.7-flash:free`) |
| **MIRAGE** | Claims free but fails | 2 (`nvidia/nemotron-3-nano-30b-a3b`, `nvidia/nemotron-3-ultra-550b-a55b`) |

**The honest takeaway:** "free" is not one thing. FOUNTAIN models genuinely
flow; THROTTLED models are free but rate-limited (the gift horse with a bridle);
MIRAGE models claim to work but fail (the worthless gift horse). **Look in the
mouth before you trust the "free" tag.**

---

## How to read this

Every number above is a real, sealed run in the Calibration Scope database
(SHA-3 provenance + the engine config used). Nothing is derived, estimated, or
marketing. The instrument is local-first: the same test battery + the same
SHA-3 seals run against local AND cloud models, cross-referenced on identical
stimulus — so you can see whether a cloud model is genuinely better or just
more expensive.

**Carrier Color** is the framework: a model's verdict tracks the CARRIER of
identical content, and the carrier sensitivity is capability-dependent. This is
the empirical pilot for the Owl Semaphore V4 metacognitive layer.
