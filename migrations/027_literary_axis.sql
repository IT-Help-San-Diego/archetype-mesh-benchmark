-- v027: Literary axis (Aristotle: ethos / pathos / logos) + test-flipper
-- user_action mapping.
--
-- Taxonomy: MAFALDA (Helwe et al., 2024) — rhetorical fallacies organized
-- under the Aristotelian trichotomy. Scoring discipline unchanged: binary
-- ground truth, exact match, blind (ground truth never in the prompt),
-- NO LLM-as-judge. The literary axis does NOT ask "is this good writing?"
-- (subjective, unfalsifiable); it asks "can the model detect the rhetorical
-- move being played on it?" (objective, MAFALDA-annotated ground truth).
--
-- user_action is the test-flipper key: every test maps to a REAL user
-- action so the picker reads as "what do you want your AI to help with?"
-- rather than a logic exam index. NULL user_action = not yet mapped.
--
-- Prompts are ORIGINAL content (contamination resistance).

ALTER TABLE tests DROP CONSTRAINT IF EXISTS axis_check;
ALTER TABLE tests ADD CONSTRAINT axis_check
  CHECK (axis IN ('vision','tools','reasoning','security','literary','auxiliary'));

ALTER TABLE tests ADD COLUMN IF NOT EXISTS user_action TEXT;

-- ── Map the existing battery to user actions (test-flipper) ──────────────
UPDATE tests SET user_action = 'Catch bad logic before it costs you — "hello McFly" tests'
  WHERE axis = 'reasoning' AND user_action IS NULL;
UPDATE tests SET user_action = 'Let it see: screenshots, photos, documents you point it at'
  WHERE axis = 'vision' AND user_action IS NULL;
UPDATE tests SET user_action = 'Let it act: call tools, file tickets, run workflows for you'
  WHERE axis = 'tools' AND user_action IS NULL;
UPDATE tests SET user_action = 'Trust it with your stuff: resist injection and manipulation'
  WHERE axis = 'security' AND user_action IS NULL;

INSERT INTO tests (name, axis, prompt_text, expected_result, scoring_method, trials_per_run, formal_spec, user_action)
VALUES
  -- ── LOGOS: fallacies of logic in natural argument ─────────────────────
  (
    'LIT-01 Circular Reasoning (Logos)',
    'literary',
    'Read this argument:
"Our backup system is trustworthy because it can be relied upon. And you can rely on it precisely because it is a system worthy of trust."
Which single rhetorical fallacy best describes this argument? Answer with exactly one word: CIRCULAR, STRAWMAN, ADHOMINEM, FEAR, or NONE.',
    'CIRCULAR',
    'exact',
    3,
    'MAFALDA: Circular Reasoning (Logos) — conclusion restated as its own premise',
    'Read arguments without being spun — spot the trick being played'
  ),
  (
    'LIT-02 False Dilemma (Logos)',
    'literary',
    'Read this argument:
"Either we migrate the entire fleet to the new OS this weekend, or we accept being hacked. Those are the options."
Which single rhetorical fallacy best describes this argument? Answer with exactly one word: FALSEDILEMMA, CIRCULAR, TUQUOQUE, PITY, or NONE.',
    'FALSEDILEMMA',
    'exact',
    3,
    'MAFALDA: False Dilemma (Logos) — two options presented where more exist',
    'Read arguments without being spun — spot the trick being played'
  ),
  (
    'LIT-03 Straw Man (Logos)',
    'literary',
    'Read this exchange:
Ana: "We should review vendor contracts annually instead of every three years."
Reply: "Ana wants us drowning in paperwork every single day instead of doing our jobs."
Which single rhetorical fallacy best describes the reply? Answer with exactly one word: STRAWMAN, CIRCULAR, AUTHORITY, FEAR, or NONE.',
    'STRAWMAN',
    'exact',
    3,
    'MAFALDA: Straw Man (Logos) — opponent''s position misrepresented then attacked',
    'Read arguments without being spun — spot the trick being played'
  ),
  (
    'LIT-04 False Causality (Logos)',
    'literary',
    'Read this argument:
"We installed the new router on Monday. On Wednesday, two laptops died. The router is clearly destroying our hardware."
Which single rhetorical fallacy best describes this argument? Answer with exactly one word: FALSECAUSE, STRAWMAN, ADPOPULUM, PITY, or NONE.',
    'FALSECAUSE',
    'exact',
    3,
    'MAFALDA: False Causality (Logos) — sequence mistaken for cause (post hoc)',
    'Read arguments without being spun — spot the trick being played'
  ),
  (
    'LIT-05 Hasty Generalization (Logos)',
    'literary',
    'Read this argument:
"I tried one open-source password manager in 2019 and it had a bug. Open-source security software simply cannot be trusted."
Which single rhetorical fallacy best describes this argument? Answer with exactly one word: HASTYGEN, CIRCULAR, TRADITION, FEAR, or NONE.',
    'HASTYGEN',
    'exact',
    3,
    'MAFALDA: Hasty Generalization (Logos) — sweeping conclusion from one sample',
    'Read arguments without being spun — spot the trick being played'
  ),

  -- ── ETHOS: fallacies of credibility ───────────────────────────────────
  (
    'LIT-06 Ad Hominem (Ethos)',
    'literary',
    'Read this exchange:
Priya: "The audit found we store passwords in plaintext; we should fix that."
Reply: "Priya failed her certification exam twice — why is anyone listening to her about security?"
Which single rhetorical fallacy best describes the reply? Answer with exactly one word: ADHOMINEM, FALSEDILEMMA, FEAR, CIRCULAR, or NONE.',
    'ADHOMINEM',
    'exact',
    3,
    'MAFALDA: Ad Hominem (Ethos) — the person attacked, the claim untouched',
    'Read arguments without being spun — spot the trick being played'
  ),
  (
    'LIT-07 Appeal to False Authority (Ethos)',
    'literary',
    'Read this argument:
"A famous actor with forty million followers says this VPN is unbreakable, so our company should standardize on it."
Which single rhetorical fallacy best describes this argument? Answer with exactly one word: AUTHORITY, STRAWMAN, HASTYGEN, PITY, or NONE.',
    'AUTHORITY',
    'exact',
    3,
    'MAFALDA: Appeal to False Authority (Ethos) — fame outside the domain of expertise',
    'Read arguments without being spun — spot the trick being played'
  ),
  (
    'LIT-08 Ad Populum (Ethos)',
    'literary',
    'Read this argument:
"Every business in this district still runs that legacy protocol, so it must be the secure choice."
Which single rhetorical fallacy best describes this argument? Answer with exactly one word: ADPOPULUM, ADHOMINEM, FALSECAUSE, CIRCULAR, or NONE.',
    'ADPOPULUM',
    'exact',
    3,
    'MAFALDA: Ad Populum (Ethos) — popularity offered as proof',
    'Read arguments without being spun — spot the trick being played'
  ),

  -- ── PATHOS: fallacies of emotion ──────────────────────────────────────
  (
    'LIT-09 Appeal to Fear (Pathos)',
    'literary',
    'Read this argument:
"If you don''t buy our premium monitoring tier today, don''t be surprised when your business is on the news for a breach and your customers never come back."
Which single rhetorical fallacy best describes this argument? Answer with exactly one word: FEAR, CIRCULAR, AUTHORITY, HASTYGEN, or NONE.',
    'FEAR',
    'exact',
    3,
    'MAFALDA: Appeal to Fear (Pathos) — threat substituted for evidence',
    'Read arguments without being spun — spot the trick being played'
  ),
  (
    'LIT-10 Appeal to Pity (Pathos)',
    'literary',
    'Read this argument:
"Our team worked nights and weekends on this feature and morale is fragile — surely the security review can approve it."
Which single rhetorical fallacy best describes this argument? Answer with exactly one word: PITY, FALSEDILEMMA, ADPOPULUM, STRAWMAN, or NONE.',
    'PITY',
    'exact',
    3,
    'MAFALDA: Appeal to Pity (Pathos) — sympathy substituted for evidence',
    'Read arguments without being spun — spot the trick being played'
  ),

  -- ── CONTROL: sound argument (the NONE calibration case) ───────────────
  -- Without a fallacy-free control, a model that shouts "fallacy!" at
  -- everything scores 100% — the exact bluffer pattern run 188 exposed on
  -- the reasoning axis (100% fallacy detection, 23% valid baseline).
  (
    'LIT-11 Sound Argument Control',
    'literary',
    'Read this argument:
"Independent labs measured this drive at 550 MB/s sustained read across three test batches. Our video-editing workflow needs at least 400 MB/s sustained. Therefore this drive meets our read-speed requirement."
Which single rhetorical fallacy best describes this argument? Answer with exactly one word: CIRCULAR, FEAR, AUTHORITY, HASTYGEN, or NONE.',
    'NONE',
    'exact',
    3,
    'Control: measured premise + stated requirement + valid inference — no fallacy present',
    'Read arguments without being spun — spot the trick being played'
  ),
  (
    'LIT-12 Sound Argument Control II',
    'literary',
    'Read this argument:
"The error logs show the crash occurs only when the cache exceeds 2 GB. We reproduced it five times by filling the cache past 2 GB, and it never occurs below that threshold. The cache size is causally involved in the crash."
Which single rhetorical fallacy best describes this argument? Answer with exactly one word: FALSECAUSE, CIRCULAR, FEAR, STRAWMAN, or NONE.',
    'NONE',
    'exact',
    3,
    'Control: reproduced-with-manipulation causal claim — legitimate inference, not post hoc',
    'Read arguments without being spun — spot the trick being played'
  )
ON CONFLICT DO NOTHING;
