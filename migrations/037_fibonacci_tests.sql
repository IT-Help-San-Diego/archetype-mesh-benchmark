-- v026: Fibonacci / recursive-sequence reasoning tests (learnable-by-watching)
-- These give the user a PERFECT, visible, learnable anchor: the recurrence
-- F(n)=F(n-1)+F(n-2) is shown in the formula panel (formal_spec), the model's
-- reasoning is shown, and the verdict is exact ground truth (55 / 21 / 6765).
-- Anti-cheat: expected_result lives ONLY here (server-side). Scoring: exact.
INSERT INTO tests (name, axis, prompt_text, expected_result, scoring_method, trials_per_run)
VALUES
  (
    'ARITH-FIB-01 Fibonacci 10th',
    'reasoning',
    'The Fibonacci sequence is defined by F(1)=1, F(2)=1, and F(n)=F(n-1)+F(n-2) for n>2.\nWhat is the 10th Fibonacci number, F(10)? Answer with only the number.',
    '55',
    'exact',
    3
  ),
  (
    'ARITH-FIB-02 Fibonacci 8th from recurrence',
    'reasoning',
    'Given the recurrence F(n)=F(n-1)+F(n-2) with F(1)=1 and F(2)=1, compute F(8).\nAnswer with only the number.',
    '21',
    'exact',
    3
  ),
  (
    'ARITH-FIB-03 Recurrence application',
    'reasoning',
    'You are told F(20)=6765 in a sequence defined by F(n)=F(n-1)+F(n-2).\nBy the definition of the sequence, what is F(19)+F(18)? Answer with only the number.',
    '6765',
    'exact',
    3
  )
ON CONFLICT DO NOTHING;

-- formal_spec: teaches the STRUCTURE, not the answer (anti-cheat discipline).
UPDATE tests SET formal_spec = 'fib : ℕ → ℕ; fib 1 = 1; fib 2 = 1; fib n = fib (n-1) + fib (n-2)'
  WHERE name = 'ARITH-FIB-01 Fibonacci 10th' AND formal_spec IS NULL;
UPDATE tests SET formal_spec = 'fib : ℕ → ℕ; fib 1 = 1; fib 2 = 1; fib n = fib (n-1) + fib (n-2)'
  WHERE name = 'ARITH-FIB-02 Fibonacci 8th from recurrence' AND formal_spec IS NULL;
UPDATE tests SET formal_spec = 'fib : ℕ → ℕ; fib n = fib (n-1) + fib (n-2)  ⇒  F(19) + F(18) = F(20)'
  WHERE name = 'ARITH-FIB-03 Recurrence application' AND formal_spec IS NULL;
