/-!
# Archetype Mesh Benchmark — Lean 4 Formalization of the Logic Battery

Every formal-logic test seeded in migrations 013/025 is stated and
machine-checked here by the Lean 4 kernel — the third independent verifier
after (1) the Python oracle (`scripts/verify_logic_ground_truth.py`,
truth tables + exhaustive small-model search) and (2) the seeded SQL
ground truth itself.

Conventions:
  * Propositional tests are stated over `Bool` and discharged by `decide`
    — the kernel literally evaluates the full truth table.
  * Valid FOL rules are proven in FULL GENERALITY (any domain `α`, any
    predicates) with explicit proof terms — stronger than the Python
    oracle, which checks finite models only.
  * Fallacies are refuted by explicit countermodels: we exhibit concrete
    predicates on `Bool` making every premise true and the conclusion
    false, so the kernel confirms the inference scheme is NOT valid.

`lean lean/ArchetypeMesh.lean` exiting 0 = every claim below is verified.
2,400 years of logic; zero trust required.
-/

namespace ArchetypeMesh

/-- Material implication on `Bool` — the truth-table connective. -/
def imp (p q : Bool) : Bool := !p || q

/-! ## Propositional layer (LOGIC-01 … LOGIC-18) — truth tables via `decide` -/

/-- LOGIC-01 Modus Ponens: `P → Q, P ⊢ Q` — VALID. -/
theorem logic01_modus_ponens :
    ∀ p q : Bool, imp p q && p → q := by decide

/-- LOGIC-02 Modus Tollens: `P → Q, ¬Q ⊢ ¬P` — VALID. -/
theorem logic02_modus_tollens :
    ∀ p q : Bool, imp p q && !q → !p := by decide

/-- LOGIC-03 Affirming the Consequent: `P → Q, Q ⊬ P` — INVALID
    (countermodel found by `decide`: p = false, q = true). -/
theorem logic03_affirming_consequent_invalid :
    ¬ (∀ p q : Bool, imp p q && q → p) := by decide

/-- LOGIC-04 Denying the Antecedent: `P → Q, ¬P ⊬ ¬Q` — INVALID. -/
theorem logic04_denying_antecedent_invalid :
    ¬ (∀ p q : Bool, imp p q && !p → !q) := by decide

/-- LOGIC-07 De Morgan: `¬(P ∧ Q) ↔ ¬P ∨ ¬Q` — VALID (equivalence). -/
theorem logic07_de_morgan :
    ∀ p q : Bool, (!(p && q)) = (!p || !q) := by decide

/-- LOGIC-08 Distribution: `P ∧ (Q ∨ R) ↔ (P ∧ Q) ∨ (P ∧ R)` — VALID. -/
theorem logic08_distribution :
    ∀ p q r : Bool, (p && (q || r)) = ((p && q) || (p && r)) := by decide

/-- LOGIC-09 Satisfiability: `(A ∨ B) ∧ (¬A ∨ C) ∧ (¬B ∨ ¬C)` — SAT.
    Witness: A = true, B = false, C = true. -/
theorem logic09_satisfiable :
    ∃ a b c : Bool, ((a || b) && (!a || c) && (!b || !c)) = true :=
  ⟨true, false, true, rfl⟩

/-- LOGIC-10 Ex falso quodlibet: `P ∧ ¬P ⊢ anything` — VALID. -/
theorem logic10_ex_falso :
    ∀ p q : Bool, p && !p → q && !q := by decide

/-- LOGIC-11 Affirming a Disjunct: `P ∨ Q, P ⊬ ¬Q` — INVALID
    (countermodel: p = true, q = true — inclusive or). -/
theorem logic11_affirming_disjunct_invalid :
    ¬ (∀ p q : Bool, (p || q) && p → !q) := by decide

/-- LOGIC-12 Denying a Conjunct: `¬(P ∧ Q), ¬P ⊬ ¬Q` — INVALID
    (countermodel: p = false, q = true; ¬(P∧Q) holds vacuously). -/
theorem logic12_denying_conjunct_invalid :
    ¬ (∀ p q : Bool, (!(p && q)) && !p → !q) := by decide

/-- LOGIC-13 Conjunctive Syllogism: `¬(P ∧ Q), P ⊢ ¬Q` — VALID.
    The deliberately-seeded VALID near-twin of LOGIC-12: the pair
    discriminates reasoning from "negative conjunction vibes". -/
theorem logic13_conjunctive_syllogism :
    ∀ p q : Bool, (!(p && q)) && p → !q := by decide

/-- LOGIC-14 Illicit Commutativity: `P → Q ⊬ Q → P` — INVALID. -/
theorem logic14_illicit_commutativity_invalid :
    ¬ (∀ p q : Bool, imp p q → imp q p) := by decide

/-- LOGIC-15 Resolution: `(P ∨ Q) ∧ (¬P ∨ R) ⊢ Q ∨ R` — VALID.
    LogicAsker's hardest valid rule (GPT-4o: 4%). -/
theorem logic15_resolution :
    ∀ p q r : Bool, (p || q) && (!p || r) → q || r := by decide

/-- LOGIC-16 Disjunctive Syllogism: `(P ∨ Q) ∧ ¬P ⊢ Q` — VALID. -/
theorem logic16_disjunctive_syllogism :
    ∀ p q : Bool, (p || q) && !p → q := by decide

/-- LOGIC-17 Constructive Dilemma: `(P→Q) ∧ (R→S) ∧ (P∨R) ⊢ Q∨S` — VALID. -/
theorem logic17_constructive_dilemma :
    ∀ p q r s : Bool, imp p q && imp r s && (p || r) → q || s := by decide

/-- LOGIC-18 Destructive Dilemma: `(P→Q) ∧ (R→S) ∧ (¬Q∨¬S) ⊢ ¬P∨¬R` — VALID. -/
theorem logic18_destructive_dilemma :
    ∀ p q r s : Bool, imp p q && imp r s && (!q || !s) → !p || !r := by decide

/-! ## First-order layer — valid rules proven over ARBITRARY domains
    (stronger than finite-model checking: these hold for every domain,
    every predicate, constructively). -/

/-- LOGIC-05 Barbara (AAA-1): `∀x(M→P), ∀x(S→M) ⊢ ∀x(S→P)` — VALID. -/
theorem logic05_barbara {α : Type} (M P S : α → Prop)
    (h₁ : ∀ x, M x → P x) (h₂ : ∀ x, S x → M x) :
    ∀ x, S x → P x :=
  fun x hs => h₁ x (h₂ x hs)

/-- LOGIC-06 Existential import: `∀x(P→Q), ∃xP ⊢ ∃xQ` — VALID. -/
theorem logic06_existential_import {α : Type} (P Q : α → Prop)
    (h : ∀ x, P x → Q x) (hex : ∃ x, P x) :
    ∃ x, Q x :=
  let ⟨w, hw⟩ := hex
  ⟨w, h w hw⟩

/-- LOGIC-27 Universal Instantiation: `∀xP(x) ⊢ P(a)` — VALID. -/
theorem logic27_universal_instantiation {α : Type} (P : α → Prop)
    (a : α) (h : ∀ x, P x) : P a :=
  h a

/-- LOGIC-28 FOL Modus Tollens: `∀x(P→Q), ¬Q(a) ⊢ ¬P(a)` — VALID. -/
theorem logic28_fol_modus_tollens {α : Type} (P Q : α → Prop)
    (a : α) (h : ∀ x, P x → Q x) (hnq : ¬ Q a) : ¬ P a :=
  fun hp => hnq (h a hp)

/-- LOGIC-29 Existential Generalization: `P(a) ⊢ ∃xP(x)` — VALID. -/
theorem logic29_existential_generalization {α : Type} (P : α → Prop)
    (a : α) (h : P a) : ∃ x, P x :=
  ⟨a, h⟩

/-! ## First-order fallacies — refuted by explicit countermodels.
    Each proof hands the kernel concrete predicates on `Bool` under which
    every premise holds and the conclusion fails. Domain size 1–2 suffices:
    monadic FOL has the finite-model property (k predicates → model ≤ 2^k). -/

/-- LOGIC-19 Existential Fallacy: `∀x(P→Q), ¬∃xP ⊬ ¬∃xQ` — INVALID.
    Countermodel: P ≡ false, Q ≡ true (Q holds for reasons other than P). -/
theorem logic19_existential_fallacy_invalid :
    ¬ (∀ (P Q : Bool → Prop),
        (∀ x, P x → Q x) → (¬ ∃ x, P x) → (¬ ∃ x, Q x)) :=
  fun h =>
    h (fun _ => False) (fun _ => True)
      (fun _ hf => hf.elim)
      (fun ⟨_, hf⟩ => hf)
      ⟨true, trivial⟩

/-- LOGIC-20 Illicit Major: `∀x(P→Q), ∃xQ ⊬ ∃xP` — INVALID. -/
theorem logic20_illicit_major_invalid :
    ¬ (∀ (P Q : Bool → Prop),
        (∀ x, P x → Q x) → (∃ x, Q x) → (∃ x, P x)) :=
  fun h =>
    let ⟨_, hp⟩ := h (fun _ => False) (fun _ => True)
      (fun _ hf => hf.elim) ⟨true, trivial⟩
    hp

/-- LOGIC-21 Undistributed Middle: `∀x(P→Q), Q(a) ⊬ P(a)` — INVALID. -/
theorem logic21_undistributed_middle_invalid :
    ¬ (∀ (P Q : Bool → Prop) (a : Bool),
        (∀ x, P x → Q x) → Q a → P a) :=
  fun h =>
    h (fun _ => False) (fun _ => True) true
      (fun _ hf => hf.elim) trivial

/-- LOGIC-22 Universal Denying the Antecedent: `∀x(P→Q), ¬P(a) ⊬ ¬Q(a)` —
    INVALID. LogicAsker: 0% detection for Gemini-1.5 / Llama3 (existential
    variant). -/
theorem logic22_universal_denying_antecedent_invalid :
    ¬ (∀ (P Q : Bool → Prop) (a : Bool),
        (∀ x, P x → Q x) → ¬ P a → ¬ Q a) :=
  fun h =>
    h (fun _ => False) (fun _ => True) true
      (fun _ hf => hf.elim) (fun hf => hf) trivial

/-- LOGIC-23 Existential Denying the Antecedent: `∃x(P→Q), ¬P(a) ⊬ ¬Q(a)` —
    INVALID. -/
theorem logic23_existential_denying_antecedent_invalid :
    ¬ (∀ (P Q : Bool → Prop) (a : Bool),
        (∃ x, P x → Q x) → ¬ P a → ¬ Q a) :=
  fun h =>
    h (fun _ => False) (fun _ => True) true
      ⟨true, fun hf => hf.elim⟩ (fun hf => hf) trivial

/-- LOGIC-24 Existential Affirming the Consequent: `∃x(P→Q), Q(a) ⊬ P(a)` —
    INVALID. -/
theorem logic24_existential_affirming_consequent_invalid :
    ¬ (∀ (P Q : Bool → Prop) (a : Bool),
        (∃ x, P x → Q x) → Q a → P a) :=
  fun h =>
    h (fun _ => False) (fun _ => True) true
      ⟨true, fun hf => hf.elim⟩ trivial

/-- LOGIC-25 Universal Affirming a Disjunct: `∀x(P∨Q), P(a) ⊬ ¬Q(a)` —
    INVALID (inclusive or: both can hold). Countermodel: P ≡ Q ≡ true. -/
theorem logic25_universal_affirming_disjunct_invalid :
    ¬ (∀ (P Q : Bool → Prop) (a : Bool),
        (∀ x, P x ∨ Q x) → P a → ¬ Q a) :=
  fun h =>
    h (fun _ => True) (fun _ => True) true
      (fun _ => Or.inl trivial) trivial trivial

/-- LOGIC-26 Universal Illicit Commutativity: `∀x(P→Q) ⊬ ∀x(Q→P)` — INVALID. -/
theorem logic26_universal_illicit_commutativity_invalid :
    ¬ (∀ (P Q : Bool → Prop),
        (∀ x, P x → Q x) → (∀ x, Q x → P x)) :=
  fun h =>
    h (fun _ => False) (fun _ => True)
      (fun _ hf => hf.elim) true trivial

end ArchetypeMesh
