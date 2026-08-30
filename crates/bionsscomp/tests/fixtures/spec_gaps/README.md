# Questions for the real compiler

Each `.nss` here is one question about the reference compiler's behaviour.
Compile it with `nwnnsscomp.exe` under Wine, read stdout, record the answer in
the file's header comment, then turn it into an assertion in
`tests/differential.rs`.

The reference compiler always exits 0 and writes every diagnostic to stdout, so
"did it work?" means "did a `.ncs` appear, and did stdout carry no error line".

## Settled by the upstream sources — confirm the KOTOR fork agrees

Torlack's NWNTools sources are at `~/Workspaces/bionsscomp_research/nwntools_upstream/`.
`_NscLib/NscParser.ypp` is the real yacc grammar and `_NscLib/NscContext.cpp`
holds the real lexer and preprocessor, so these are no longer open questions
about *the language*. They stay here because the KOTOR build is a fork by
Hazard and by tk102 for stoffe, and a fork can change anything. Each one is a
cheap check that the fork did not.

| File | Question | Upstream answer, and what this implementation does |
|---|---|---|
| `float_forms.nss` | Which float spellings does the lexer take? | Digits, at most one `.`, optional trailing `f`/`F` that is not part of the converted text. So `1.` is `1.0`, `5f` is `5.0`, and `1.5e3` is the float `1.5` followed by the identifier `e3`. No exponent, no leading dot. This contradicts the bytecode spec's §1.1, which guessed at C-style scanning. |
| `for_init_decl.nss` | Does `for (int i = 0; ...)` compile? | No. `for_start` takes expressions in all three slots and there is no declaration production. Syntax error. |
| `member_call.nss` | Can a call be spelled on a struct member? | No. The call productions are `IDENTIFIER '(' ... ')'` hanging off `primary_expression`. Syntax error. |
| `param_trailing_comma.nss` | Is `void f(int a,)` accepted? | Yes. `function_parameter_type_list` has a `function_parameter_list ','` alternative. |
| `vector_literal.nss` | Does the bracket vector constant exist? | Yes, with four productions: `[]`, `[f]`, `[f,f]`, `[f,f,f]`, float literals only, missing components zero. This contradicts the bytecode spec, which called it a PyKotor invention. |
| `assign_lhs.nss` | Is `a + b = c` a syntax error or a sema error? | The assignment production takes a `unary_expression` on the left, so an additive expression cannot reduce: syntax error at `"="`. Worth checking anyway, because the reference compiler also carries the string `Left hand side of assignment not a variable` and LALR lookahead can surprise. |
| `hash_in_comment.nss` | Is a directive inside a block comment a directive? | No. `ReadNextLine(fInComment)` skips preprocessing while the token fetch loop is inside a comment. |

## Genuinely open

Nothing in the sources answers these. The first two are KOTOR-fork behaviour
with no upstream equivalent; the third is a code generation question.

| File | Question | What this implementation does |
|---|---|---|
| `decimal_overflow.nss` | What does an out-of-range decimal literal become? | The lexer runs the digits through `atol`, whose overflow behaviour is not wrapping and is not specified. This saturates to `2147483647`. Hex is separate and wraps, because it accumulates with plain 32-bit multiply and add. |
| `include_forward_slash.nss` | Does a forward-slash include path resolve? | Accepted. Backslash is verified to work. Forward slash was never tested, and the fork's extension stripping resets on `\` only, which hints it was never considered. A Linux port has to take both. |
| `engine_type_qualifier.nss` | Which NCS qualifier byte does an engine structure get? | `0x10 + n` for one operand, `0x30 + n` for a pair. The two specs disagree; retail bytecode contains `RSADDEFF` (`0x02 0x10`), which favours this reading. Compile and disassemble with `-d` to settle it. |
