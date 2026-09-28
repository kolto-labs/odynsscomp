// SPEC-GAP: vector-literal
//
// Question: does the bracket vector constant exist, and how many components
// may it have?
//
// The bytecode spec calls `[x, y, z]` a PyKotor invention that `nwnnsscomp`
// does not have. The yacc grammar disagrees: it has four productions, `[]`,
// `[f]`, `[f,f]` and `[f,f,f]`, with float literals only. This implementation
// follows the grammar. Settle it, because getting this wrong in either
// direction changes which scripts compile.
//
// Answer from the real compiler: <record here>

void main()
{
    vector vFull = [1.0, 2.0, 3.0];
    vector vFlat = [1.0, 2.0];
    vector vOne = [1.0];
    vector vZero = [];
}
