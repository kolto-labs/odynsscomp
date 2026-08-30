// SPEC-GAP: for-init-declaration
//
// Question: does a declaration in a for initializer compile?
//
// The yacc grammar's for_start takes only expressions in all three slots.
// There is no production for a declaration. So this must fail. It matters
// because accepting it would let this compiler build scripts the real one
// rejects, which is the failure mode that hurts most: a mod that builds here
// and not for anyone else.
//
// Answer from the real compiler: <record here>

void main()
{
    for (int i = 0; i < 3; i++)
    {
    }
}
