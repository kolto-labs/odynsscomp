// SPEC-GAP: parameter-trailing-comma
//
// Question: is a trailing comma allowed in a parameter list?
//
// The grammar says yes: `function_parameter_type_list` has a
// `function_parameter_list ','` alternative. That is unusual enough to be
// worth confirming against the KOTOR build, which is a different version of
// the source.
//
// Answer from the real compiler: <record here>

void Report(int nStep, int nTotal,)
{
    nStep = nTotal;
}

void main()
{
    Report(1, 2);
}
