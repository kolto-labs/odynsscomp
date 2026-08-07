// Include resolution fixture. Written for this repository. Not game data.
//
// The backslash path is the point: existing mod source is full of them and
// they have to resolve on Linux too.
#include "inc_a"
#include "incs\inc_b"
// The extension is implied, so this names the same file as the first include
// and must not be pulled in twice.
#include "inc_a.nss"

void main()
{
    AlphaThing();
    BetaThing();
}
