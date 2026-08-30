// SPEC-GAP: member-call
//
// Question: can a call be spelled on anything other than a bare identifier?
//
// The grammar's call productions are `IDENTIFIER '(' ... ')'` only, hanging off
// primary_expression rather than postfix_expression. So `s.fn()` should not
// parse. Confirm, because NWScript has no function pointers and nobody would
// write this on purpose — but a struct member named like a function might turn
// up in generated source.
//
// Answer from the real compiler: <record here>

struct Holder
{
    int nValue;
};

void main()
{
    struct Holder s;
    s.nValue();
}
