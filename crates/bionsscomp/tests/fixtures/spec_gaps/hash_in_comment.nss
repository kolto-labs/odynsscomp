// SPEC-GAP: directive-inside-comment
//
// Question: is a directive inside a block comment still a directive?
//
// The reference compiler preprocesses inside the token fetch loop and skips
// preprocessing while the loop is inside a block comment. So the `#define`
// below should be plain comment text and the file should compile. If it
// instead reports `#define unknown or invalid definition`, this
// implementation's comment tracking is wrong.
//
// Answer from the real compiler: <record here>

/*
#define MY_CONST 42
#include "no_such_include"
*/

void main()
{
}
