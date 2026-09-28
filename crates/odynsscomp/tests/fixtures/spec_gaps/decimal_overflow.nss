// SPEC-GAP: decimal-literal-overflow
//
// Question: what does a decimal literal larger than a 32-bit int become?
//
// The reference lexer runs the digits through `atol`. MSVC's `atol` on
// overflow is not the same as wrapping. This implementation saturates to
// 2147483647. Hex is separate: it accumulates with plain 32-bit multiply and
// add, so it wraps.
//
// How to settle it: compile, disassemble with -d, read the CONSTI payloads.
//
// Answer from the real compiler: <record here>

void main()
{
    int nBig = 2147483648;      // one past the top
    int nHuge = 99999999999;    // far past it
    int nHexWrap = 0x1FFFFFFFF; // 33 bits of hex
}
