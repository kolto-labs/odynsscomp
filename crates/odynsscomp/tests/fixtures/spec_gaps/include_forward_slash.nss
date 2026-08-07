// SPEC-GAP: include-forward-slash
//
// Question: does a forward-slash include path resolve?
//
// Backslash subpaths are verified to work. Forward slashes were never tested,
// and the reference compiler's extension stripping explicitly resets on `\`
// only, which hints that `/` was never considered. This implementation accepts
// both, because a Linux port has to.
//
// This file needs a sibling at incs/inc_b.nss to resolve against. Copy it next
// to tests/fixtures/includes/ before running.
//
// Answer from the real compiler: <record here>

#include "incs/inc_b"

void main()
{
    BetaThing();
}
