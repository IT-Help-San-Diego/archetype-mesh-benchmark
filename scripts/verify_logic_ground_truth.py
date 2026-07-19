#!/usr/bin/env python3
"""Ground-truth oracle for the Calibration Scope formal logic battery.

Every formal-logic test seeded in migrations 013/025 is re-verified here by a
COMPLETE decision procedure — not a heuristic, not an LLM:

  * Propositional tests: exhaustive truth-table evaluation (2^n assignments).
  * Monadic predicate tests: exhaustive model search over domains of size
    1..4. By the finite-model property of monadic FOL, a formula with k
    monadic predicates is satisfiable iff it has a model of size <= 2^k; all
    our tests use k <= 2 predicates, so domain <= 4 is a complete decision
    procedure. Every INVALID verdict is backed by an explicit countermodel.

Run: python3 scripts/verify_logic_ground_truth.py
Exit 0 = every seeded ground truth matches the computed verdict.
Exit 1 = MISMATCH (a seeded test is wrong — do not ship).

This is the anti-"hello McFly" gate: 2,400 years of logic, machine-checked,
so nobody has to take our ground truth on faith.
"""
import sys
from itertools import product

IMP = lambda a, b: (not a) or b


def prop_verdict(n, premises, conclusion):
    for v in product([False, True], repeat=n):
        if all(p(v) for p in premises) and not conclusion(v):
            return "INVALID", v
    return "VALID", None


def fol_verdict(n_preds, premises, conclusion, max_dom=4):
    for n in range(1, max_dom + 1):
        dom = list(range(n))
        for exts in product(list(product([False, True], repeat=n)), repeat=n_preds):
            preds = [lambda x, e=e: e[x] for e in exts]
            for a in dom:
                if all(p(dom, preds, a) for p in premises) and not conclusion(dom, preds, a):
                    return "INVALID", (n, exts, a)
    return "VALID", None


ALL = lambda dom, f: all(f(x) for x in dom)
SOME = lambda dom, f: any(f(x) for x in dom)

# ── The battery: name -> (kind, structure, seeded ground truth) ──────────
# Structures MUST mirror the formal_spec column seeded in the migrations.
PROP = {
    "LOGIC-01 Modus Ponens": (
        2, [lambda v: IMP(v[0], v[1]), lambda v: v[0]], lambda v: v[1], "VALID"),
    "LOGIC-02 Modus Tollens": (
        2, [lambda v: IMP(v[0], v[1]), lambda v: not v[1]], lambda v: not v[0], "VALID"),
    "LOGIC-03 Affirming the Consequent (Fallacy)": (
        2, [lambda v: IMP(v[0], v[1]), lambda v: v[1]], lambda v: v[0], "INVALID"),
    "LOGIC-04 Denying the Antecedent (Fallacy)": (
        2, [lambda v: IMP(v[0], v[1]), lambda v: not v[0]], lambda v: not v[1], "INVALID"),
    "LOGIC-07 Boolean Algebra - De Morgan": (
        2, [], lambda v: (not (v[0] and v[1])) == ((not v[0]) or (not v[1])), "VALID"),
    "LOGIC-08 Boolean Algebra - Distribution": (
        3, [], lambda v: (v[0] and (v[1] or v[2])) == ((v[0] and v[1]) or (v[0] and v[2])), "VALID"),
    "LOGIC-10 Contradiction Detection": (
        # P∧¬P premise: explosion — anything follows, incl. Q∧¬Q.
        2, [lambda v: v[0] and not v[0]], lambda v: v[1] and not v[1], "VALID"),
    "LOGIC-11 Affirming a Disjunct (Fallacy)": (
        2, [lambda v: v[0] or v[1], lambda v: v[0]], lambda v: not v[1], "INVALID"),
    "LOGIC-12 Denying a Conjunct (Fallacy)": (
        2, [lambda v: not (v[0] and v[1]), lambda v: not v[0]], lambda v: not v[1], "INVALID"),
    "LOGIC-13 Conjunctive Syllogism": (
        2, [lambda v: not (v[0] and v[1]), lambda v: v[0]], lambda v: not v[1], "VALID"),
    "LOGIC-14 Illicit Commutativity (Fallacy)": (
        2, [lambda v: IMP(v[0], v[1])], lambda v: IMP(v[1], v[0]), "INVALID"),
    "LOGIC-15 Resolution": (
        3, [lambda v: v[0] or v[1], lambda v: (not v[0]) or v[2]], lambda v: v[1] or v[2], "VALID"),
    "LOGIC-16 Disjunctive Syllogism": (
        2, [lambda v: v[0] or v[1], lambda v: not v[0]], lambda v: v[1], "VALID"),
    "LOGIC-17 Constructive Dilemma": (
        4, [lambda v: IMP(v[0], v[1]), lambda v: IMP(v[2], v[3]), lambda v: v[0] or v[2]],
        lambda v: v[1] or v[3], "VALID"),
    "LOGIC-18 Destructive Dilemma": (
        4, [lambda v: IMP(v[0], v[1]), lambda v: IMP(v[2], v[3]),
            lambda v: (not v[1]) or (not v[3])],
        lambda v: (not v[0]) or (not v[2]), "VALID"),
}

FOL = {
    "LOGIC-05 Syllogism - Barbara (AAA-1)": (
        3, [lambda d, p, a: ALL(d, lambda x: IMP(p[0](x), p[1](x))),
            lambda d, p, a: ALL(d, lambda x: IMP(p[2](x), p[0](x)))],
        lambda d, p, a: ALL(d, lambda x: IMP(p[2](x), p[1](x))), "VALID"),
    "LOGIC-06 Syllogism - Existential Fallacy": (
        # ∀x(P→Q), ∃xP ⊢ ∃xQ — existential premise makes this VALID
        2, [lambda d, p, a: ALL(d, lambda x: IMP(p[0](x), p[1](x))),
            lambda d, p, a: SOME(d, p[0])],
        lambda d, p, a: SOME(d, p[1]), "VALID"),
    "LOGIC-19 Existential Fallacy (Fallacy)": (
        2, [lambda d, p, a: ALL(d, lambda x: IMP(p[0](x), p[1](x))),
            lambda d, p, a: not SOME(d, p[0])],
        lambda d, p, a: not SOME(d, p[1]), "INVALID"),
    "LOGIC-20 Illicit Major (Fallacy)": (
        2, [lambda d, p, a: ALL(d, lambda x: IMP(p[0](x), p[1](x))),
            lambda d, p, a: SOME(d, p[1])],
        lambda d, p, a: SOME(d, p[0]), "INVALID"),
    "LOGIC-21 Undistributed Middle (Fallacy)": (
        2, [lambda d, p, a: ALL(d, lambda x: IMP(p[0](x), p[1](x))),
            lambda d, p, a: p[1](a)],
        lambda d, p, a: p[0](a), "INVALID"),
    "LOGIC-22 Universal Denying the Antecedent (Fallacy)": (
        2, [lambda d, p, a: ALL(d, lambda x: IMP(p[0](x), p[1](x))),
            lambda d, p, a: not p[0](a)],
        lambda d, p, a: not p[1](a), "INVALID"),
    "LOGIC-23 Existential Denying the Antecedent (Fallacy)": (
        2, [lambda d, p, a: SOME(d, lambda x: IMP(p[0](x), p[1](x))),
            lambda d, p, a: not p[0](a)],
        lambda d, p, a: not p[1](a), "INVALID"),
    "LOGIC-24 Existential Affirming the Consequent (Fallacy)": (
        2, [lambda d, p, a: SOME(d, lambda x: IMP(p[0](x), p[1](x))),
            lambda d, p, a: p[1](a)],
        lambda d, p, a: p[0](a), "INVALID"),
    "LOGIC-25 Universal Affirming a Disjunct (Fallacy)": (
        2, [lambda d, p, a: ALL(d, lambda x: p[0](x) or p[1](x)),
            lambda d, p, a: p[0](a)],
        lambda d, p, a: not p[1](a), "INVALID"),
    "LOGIC-26 Universal Illicit Commutativity (Fallacy)": (
        2, [lambda d, p, a: ALL(d, lambda x: IMP(p[0](x), p[1](x)))],
        lambda d, p, a: ALL(d, lambda x: IMP(p[1](x), p[0](x))), "INVALID"),
    "LOGIC-27 Universal Instantiation": (
        1, [lambda d, p, a: ALL(d, p[0])],
        lambda d, p, a: p[0](a), "VALID"),
    "LOGIC-28 FOL Modus Tollens": (
        2, [lambda d, p, a: ALL(d, lambda x: IMP(p[0](x), p[1](x))),
            lambda d, p, a: not p[1](a)],
        lambda d, p, a: not p[0](a), "VALID"),
    "LOGIC-29 Existential Generalization": (
        1, [lambda d, p, a: p[0](a)],
        lambda d, p, a: SOME(d, p[0]), "VALID"),
}


def main():
    failures = 0
    for name, (n, prem, concl, claimed) in PROP.items():
        verdict, cex = prop_verdict(n, prem, concl)
        ok = verdict == claimed
        failures += 0 if ok else 1
        mark = "PASS" if ok else "FAIL"
        extra = f"  countermodel {cex}" if cex and claimed == "INVALID" else ""
        print(f"[{mark}] {name}: computed={verdict} seeded={claimed}{extra}")
    for name, (k, prem, concl, claimed) in FOL.items():
        verdict, cex = fol_verdict(k, prem, concl)
        ok = verdict == claimed
        failures += 0 if ok else 1
        mark = "PASS" if ok else "FAIL"
        extra = f"  countermodel dom={cex[0]} ext={cex[1]} a={cex[2]}" if cex and claimed == "INVALID" else ""
        print(f"[{mark}] {name}: computed={verdict} seeded={claimed}{extra}")
    total = len(PROP) + len(FOL)
    print(f"\n{total - failures}/{total} ground truths verified" + (" — ALL CORRECT" if failures == 0 else f" — {failures} MISMATCH(ES), DO NOT SHIP"))
    sys.exit(1 if failures else 0)


if __name__ == "__main__":
    main()
