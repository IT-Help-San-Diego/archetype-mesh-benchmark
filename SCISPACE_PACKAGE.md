# SciSpace Research Package — Calibration Scope
## For Scientific Paper Discovery & Citation Mapping

### Project Summary (for SciSpace bot context)

We are building a scientific LLM capability benchmark called "Calibration Scope" that tests whether local and cloud LLMs can actually perform agentic tasks — Vision, Tool Use, Reasoning, and Security — on the user's own hardware. Unlike existing benchmarks (MMLU, HELM, Chatbot Arena, lm-eval-harness), our system tests models locally via LM Studio with clean-room execution (eject all models, load only target, verify RAM residency before testing), blind testing (ground truth never sent to model), SHA-3 cryptographic provenance on every trial, and objective scoring (no LLM-as-judge). It also measures speculative decoding performance (draft model acceptance rates and speedup factors) and latency alongside accuracy — features no existing benchmark provides.

### Key Research Questions (papers we need)

1. **LLM Logical Reasoning & Fallacy Detection**
   - How do existing benchmarks test logical reasoning in LLMs?
   - Which benchmarks test fallacy detection (affirming the consequent, denying the antecedent)?
   - What patterns of reasoning failure are documented across models?
   - Papers: LogicBench, LogicAsker, FOLIO, RuozhiBench, LoFa, MAFALDA, ReClor, AR-LSAT, ProofWriter

2. **Benchmark Contamination & Test Set Memorization**
   - How do models memorize benchmark test sets?
   - What methods detect and mitigate contamination?
   - Papers: LiveBench, CoDeC, n-gram matching, MinHash detection methods

3. **Speculative Decoding Performance Measurement**
   - How is draft token acceptance rate measured and optimized?
   - What draft model pairing strategies work best?
   - When does speculative decoding help vs hurt?
   - Papers: speculative decoding acceptance rate, draft model selection, multi-draft spec decode

4. **Model Routing & Selection Based on Capability**
   - How do systems route tasks to the right model based on capability?
   - Papers: RouteLLM, FrugalGPT, Router-R1, Eagle Router, SELECT-THEN-ROUTE

5. **Vision-Language Reasoning (Can models REASON about what they see?)**
   - Do existing benchmarks test logical reasoning about visual content?
   - Papers: VisualPuzzles, MMBench, VSR, spatial reasoning benchmarks

6. **Local LLM Evaluation on Consumer Hardware**
   - Are there benchmarks designed for models running on consumer hardware?
   - How do hardware constraints (RAM, GPU) affect model capability?
   - Papers: Ollama benchmarks, LM Studio evaluation, local model performance studies

7. **Agentic Benchmarks (Tool Use, Multi-Step Reasoning)**
   - How are agent capabilities (tool use, multi-step reasoning) evaluated?
   - Papers: SWE-bench, AgentBench, WebArena, GAIA, BrowserGym

8. **Security & Trustworthiness in LLM Evaluation**
   - How is LLM security (prompt injection resistance, system prompt extraction) tested?
   - Papers: HELM trustworthiness axis, security benchmarks, jailbreak resistance

### Our Unique Contribution (for citation context)

Our benchmark combines 9 features that no single existing benchmark provides:
1. Tests local models on user hardware (not cloud APIs)
2. Clean-room execution (eject/load/verify before testing)
3. Blind testing (ground truth hidden from model)
4. SHA-3 cryptographic provenance on results
5. Speculative decoding measurement (draft acceptance rates)
6. Latency measurement alongside accuracy for local models
7. Tests Vision + Tools + Reasoning + Security in one framework
8. Memory guard for hardware safety
9. Objective scoring (no LLM-as-judge, no subjective preference)

### Key Findings From Our Data (for paper context)

- **Universal Fallacy Blindness**: 19 of 21 tested local LLMs systematically endorse classic logical fallacies (affirming the consequent → VALID, denying the antecedent → VALID). Only gpt-oss-20b (local) and Claude Fable 5 (cloud) pass all 33 reasoning tests.
- **Bigger ≠ Better**: hermes-4-70b (35GB, 120s/response) scored worse on reasoning (partial, 15/19) than gpt-oss-20b (11GB, 1.2s/response, 33/33 perfect).
- **Security Bloodbath**: 14 of 21 models fail system prompt extraction. The best reasoning model (gpt-oss-20b, 33/33) fails security (0/3).
- **Speculative Decoding**: 3x speedup with 88% draft acceptance on gemma-4-31b + gemma-4-12b-qat pair (GGUF/llama.cpp). MLX models cannot use spec decode (batched MLX limitation).

### Technology Stack
- Backend: Rust (axum 0.8.9 + tokio + sqlx 0.8 + PostgreSQL 18)
- Frontend: Single-page HTML with SSE (Server-Sent Events) for live telemetry
- Local inference: LM Studio REST API (OpenAI-compatible)
- Cloud inference: Nous Portal, OpenRouter
- Provenance: SHA-3-512 hashing of evidence records
- Deployment: macOS launchd service

### Repository
https://github.com/IT-Help-San-Diego/calibration-scope

### Authors
Carey James Balboa — IT Help San Diego Inc.
Research program: Intellectual Resistance (intellectualresistance.com)

### Search Terms for SciSpace
"LLM benchmark" "logical reasoning" "fallacy detection" "speculative decoding" "local model evaluation" "blind testing" "model capability" "agentic benchmark" "vision reasoning" "tool use evaluation" "provenance" "contamination resistance" "model routing" "clean room" "LM Studio" "consumer hardware" "SHA-3" "evidence"
