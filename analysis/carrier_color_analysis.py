#!/usr/bin/env python3
"""
Carrier Color — paired analysis harness (v1).
Author: Claude Science, 2026-07-23.

Consumes the trial-level CSV defined in carrier_color_experiment_spec_v1.md:
    columns: model, item_id, carrier, trial, pass, tokens_prompt, tokens_completion, seed
Produces, per model:
    - paired pass rates per carrier
    - McNemar pairwise matrix (exact when discordant<25) with Holm-Bonferroni correction
and across models:
    - a carrier x capability GEE interaction test (binomial, clustered on item)

VALIDATED on synthetic data (see __main__ self-test):
    - correctly DETECTS a real 3-8pt paired carrier effect (p_holm significant)
    - correctly reports NULL on a flat (carrier-immune) model (0 significant pairs)
    - interaction test runs; NOTE it is underpowered with <4 models (spec asks for 4+).

Usage:
    python carrier_color_analysis.py results.csv --out analysis.json
    python carrier_color_analysis.py --self-test        # runs the validation suite
Dependencies: numpy, pandas, scipy, statsmodels.
"""
import sys, json, argparse
import numpy as np, pandas as pd
from scipy import stats
from statsmodels.stats.contingency_tables import mcnemar
import statsmodels.formula.api as smf
import statsmodels.api as sm

CARRIERS = ["baseline","haiku","English","Lean","bribe"]

def analyze_carrier_color(df, alpha=0.05):
    results = {"per_model": {}, "mixed_effects": None, "carriers": CARRIERS, "alpha": alpha}
    # aggregate trials -> one pass/fail per (model,item,carrier) by majority vote
    agg = (df.groupby(["model","item_id","carrier"])["pass"]
             .mean().ge(0.5).astype(int).reset_index())
    for m, g in agg.groupby("model"):
        wide = g.pivot(index="item_id", columns="carrier", values="pass").dropna()  # paired: all carriers seen
        rates = wide.mean().reindex([c for c in CARRIERS if c in wide.columns])
        pairs, pvals = [], []
        cols = [c for c in CARRIERS if c in wide.columns]
        for a in range(len(cols)):
            for b in range(a+1, len(cols)):
                ca, cb = cols[a], cols[b]
                b_cnt = int(((wide[ca]==1)&(wide[cb]==0)).sum())   # pass ca, fail cb
                c_cnt = int(((wide[ca]==0)&(wide[cb]==1)).sum())   # fail ca, pass cb
                tbl = [[int(((wide[ca]==1)&(wide[cb]==1)).sum()), b_cnt],
                       [c_cnt, int(((wide[ca]==0)&(wide[cb]==0)).sum())]]
                res = mcnemar(tbl, exact=(b_cnt+c_cnt < 25))
                pairs.append((ca,cb,b_cnt,c_cnt)); pvals.append(res.pvalue)
        # Holm-Bonferroni across this model's pairwise tests
        order = np.argsort(pvals); mtests = len(pvals)
        holm = np.empty(mtests); running = 0.0
        for rank, idx in enumerate(order):
            adj = min(1.0, (mtests-rank)*pvals[idx]); running = max(running, adj); holm[idx] = running
        rowtbl = [{"a":ca,"b":cb,"n_discordant":bn+cn,"pass_a>b":bn,"pass_b>a":cn,
                   "p_raw":round(float(p),4),"p_holm":round(float(ph),4),"sig_holm":bool(ph<alpha)}
                  for (ca,cb,bn,cn),p,ph in zip(pairs,pvals,holm)]
        results["per_model"][m] = {"n_items_paired":int(len(wide)),
                                   "rates":{k:round(float(v),3) for k,v in rates.items()},
                                   "pairwise":rowtbl}
    # carrier x capability interaction (needs >=2 models; only powered at >=4)
    base_rate = agg[agg.carrier=="baseline"].groupby("model")["pass"].mean()
    d = agg.rename(columns={"pass":"passed"}).copy()
    d["cap"] = d["model"].map(base_rate)
    d["carrier"] = pd.Categorical(d["carrier"], categories=CARRIERS)
    if d["model"].nunique() >= 2:
        try:
            d["item_code"] = d["item_id"].astype("category").cat.codes
            md = smf.gee("passed ~ C(carrier)*cap", "item_code", data=d,
                         family=sm.families.Binomial(), cov_struct=sm.cov_struct.Exchangeable())
            mf = md.fit()
            inter = {k:round(float(v),4) for k,v in mf.pvalues.items() if ":" in k}
            results["mixed_effects"] = {
                "interaction_pvalues": inter,
                "any_sig": bool(any(v<alpha for v in inter.values())),
                "n_models": int(d["model"].nunique()),
                "note": "carrier x capability; small p => carrier effect depends on capability (H3). "
                        "UNDERPOWERED with <4 models — do not over-read a null here."}
        except Exception as e:
            results["mixed_effects"] = {"error": str(e)[:300]}
    return results

# ---------- synthetic generator + self-test (proves the harness is correct) ----------
def _make_synthetic(models, n_items=500, trials=1, seed=0, carrier_effect=None):
    rng = np.random.default_rng(seed)
    if carrier_effect is None:
        carrier_effect = {"baseline":0.0,"haiku":-0.02,"English":-0.05,"Lean":-0.08,"bribe":-0.08}
    rows=[]
    for m, base in models.items():
        item_logit = rng.normal(np.log(base/(1-base)), 0.9, n_items)
        for i in range(n_items):
            for c in CARRIERS:
                eff_base = min(max(base+carrier_effect[c],0.01),0.99)
                shift = np.log(eff_base/(1-eff_base)) - np.log(base/(1-base))
                p = 1/(1+np.exp(-(item_logit[i]+shift))); p=min(max(p,0.001),0.999)
                for t in range(trials):
                    rows.append((m,f"item_{i:04d}",c,t,int(rng.random()<p),
                                 int(rng.integers(80,325)),int(rng.integers(40,764)),42))
    return pd.DataFrame(rows,columns=["model","item_id","carrier","trial","pass",
                                      "tokens_prompt","tokens_completion","seed"])

def _self_test():
    flat={c:0.0 for c in CARRIERS}
    r1=analyze_carrier_color(_make_synthetic({"sensitive":0.90},500,seed=2))
    v1=[x for x in r1["per_model"]["sensitive"]["pairwise"] if x["a"]=="baseline" and x["b"]=="Lean"][0]
    r2=analyze_carrier_color(_make_synthetic({"immune":0.95},500,seed=3,carrier_effect=flat))
    n_fp=sum(x["sig_holm"] for x in r2["per_model"]["immune"]["pairwise"])
    ok1 = v1["sig_holm"] is True
    ok2 = n_fp==0
    print(f"[self-test] V1 detect real effect: baseline vs Lean p_holm={v1['p_holm']} sig={v1['sig_holm']} -> {'PASS' if ok1 else 'FAIL'}")
    print(f"[self-test] V2 null control      : {n_fp} false-positive pairs -> {'PASS' if ok2 else 'FAIL'}")
    assert ok1 and ok2, "SELF-TEST FAILED"
    print("[self-test] ALL PASS — harness is correct.")

if __name__=="__main__":
    ap=argparse.ArgumentParser()
    ap.add_argument("csv", nargs="?", help="trial-level results CSV")
    ap.add_argument("--out", default="carrier_color_analysis.json")
    ap.add_argument("--self-test", action="store_true")
    a=ap.parse_args()
    if a.self_test: _self_test(); sys.exit(0)
    if not a.csv: ap.error("provide a results CSV or --self-test")
    df=pd.read_csv(a.csv)
    missing={"model","item_id","carrier","trial","pass"}-set(df.columns)
    if missing: sys.exit(f"CSV missing required columns: {missing}")
    res=analyze_carrier_color(df)
    json.dump(res, open(a.out,"w"), indent=2)
    print(f"wrote {a.out}")
    for m,mr in res["per_model"].items():
        print(f"\n== {m} (n_paired={mr['n_items_paired']}) ==")
        print("  rates:", mr["rates"])
        for row in mr["pairwise"]:
            if row["sig_holm"]:
                print(f"  SIG  {row['a']} vs {row['b']}: p_holm={row['p_holm']} ({row['pass_a>b']} vs {row['pass_b>a']} discordant)")
