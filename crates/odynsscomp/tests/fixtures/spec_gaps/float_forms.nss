// SPEC-GAP: float-literal-grammar
//
// Question: which float spellings does the lexer accept, and where does a
// literal end?
//
// This implementation reads the reference lexer as taking digits, at most one
// `.`, and an optional trailing `f`/`F` that is not part of the converted
// text. That makes every line below well defined and makes `1e3` two tokens.
// The bytecode spec's §1.1 guesses at C-style scanning with an exponent, which
// would make `fExp` 1000.0 instead of a syntax error.
//
// How to settle it: compile, then disassemble with -d and read the CONSTF
// payloads. If `fExp` does not compile at all, that also answers it.
//
// Answer from the real compiler: <record here>

void main()
{
    float fDot = 1.;        // expected 1.0
    float fSuffix = 5f;     // expected 5.0
    float fBoth = 1.5f;     // expected 1.5
    float fExp = 1.5e3;     // expected: float 1.5, then identifier e3 -> error
}
