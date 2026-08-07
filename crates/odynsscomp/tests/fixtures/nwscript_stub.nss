// A stand-in for the game's nwscript.nss, written for this repository.
//
// The real file is game data and is not redistributable, so it is never
// committed here. This holds just enough shape to exercise the loader: a
// couple of prototypes, one with a default argument, and one constant.
//
// Routine ids come from position in the file, so the ids below are 0 and 1.

int TRUE = 1;

// id 0
void PrintString(string sString);

// id 1
int Random(int nMaxInteger, int nSeed = 0);
