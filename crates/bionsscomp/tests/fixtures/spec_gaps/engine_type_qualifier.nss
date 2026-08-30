// SPEC-GAP: engine-type-qualifier
//
// Question: which NCS qualifier byte does an instruction on an engine
// structure carry?
//
// The two specs disagree. The behavioural spec reads the reference compiler's
// single-type mapping as sending everything that is not int, float, string or
// object to 0x00. The bytecode spec says engine structures use 0x10 + n for a
// single operand and 0x30 + n for a pair. Retail bytecode contains RSADDEFF
// (0x02 0x10), which favours the second reading, and that is what
// EngineType::unary_type_code returns.
//
// This is a code generation question, not a parsing one. It is filed here so
// it is not lost.
//
// How to settle it: compile, disassemble with -d, and read the qualifier byte
// on the RSADD that reserves eLocal.
//
// Answer from the real compiler: <record here>

effect GetSomeEffect();

void main()
{
    effect eLocal = GetSomeEffect();
}
