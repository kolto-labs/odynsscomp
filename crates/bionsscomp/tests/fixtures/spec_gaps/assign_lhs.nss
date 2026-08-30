// SPEC-GAP: assignment-lhs
//
// Question: is assigning to a non-lvalue a syntax error or a semantic one?
//
// The yacc grammar's assignment production takes a unary_expression on the
// left, so `a + b = c` cannot reduce and must be `Syntax error at "="`. But
// the reference compiler also carries the message
// `Left hand side of assignment not a variable`, which has to come from
// somewhere. Find out which one this produces.
//
// Answer from the real compiler: <record here>

void main()
{
    int a, b, c;
    a + b = c;
}
