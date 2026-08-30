// Every construct in this file is legal NWScript that a C programmer would
// read wrong. Written for this repository. Not game data.
//
// tests/frontend.rs pins each one down.

#define ENGINE_NUM_STRUCTURES 4
#define ENGINE_STRUCTURE_0 effect
#define ENGINE_STRUCTURE_1 event
#define ENGINE_STRUCTURE_2 location
#define ENGINE_STRUCTURE_3 talent

struct Waypoint
{
    location lWhere;
    string sTag, sComment;
};

// INT and OBJECT_ID are keywords, not typedefs.
INT gnCounter = 0;
OBJECT_ID goTarget = OBJECT_INVALID;

const int MAX_TRIES = 3;

// A declarator list is several declarations.
float gfX = 1.0, gfY, gfZ = 2.5f;

// A prototype has no body. Inside nwscript.nss that makes it an engine
// routine; here it is a forward declaration.
void ReportProgress(string sLabel, int nStep = 1, int nTotal = MAX_TRIES,);

void ReportProgress(string sLabel, int nStep, int nTotal,)
{
    // The only escape is \n. A backslash before anything else is dropped, so
    // this string reads "Step: " and then a real newline.
    string sMessage = "Step:\t" + sLabel + "\n";

    // Floats take one dot and an optional f. No exponent, no leading dot.
    float fRatio = 1.;
    float fHalf = 0.5f;
    float fFive = 5f;

    // >>> is unsigned right shift. & | ^ are integer bitwise operators.
    int nMask = (gnCounter >>> 2) & 0xFF | 0x0100 ^ 1;
    nMask >>>= 1;

    // A vector constant. The components are float literals and missing ones
    // are zero.
    vector vHere = [1.0, 2.0, 3.0];
    vector vFlat = [1.0, 2.0];
    vector vZero = [];

    struct Waypoint wp;
    wp.sTag = sLabel;
    wp.lWhere = GetStartingLocation();

    if (nStep >= nTotal)
        gnCounter = 0;
    else
        gnCounter++;

    // for takes expressions in every slot, never a declaration.
    int i;
    for (i = 0; i < nTotal; i++)
    {
        if (i == nStep) continue;
        if (i > 10) break;
    }

    switch (nStep)
    {
        case 1:
        case 2:
            gnCounter += 1;
            break;
        default:
            gnCounter = -gnCounter;
            break;
    }

    do
    {
        i--;
    } while (i > 0);

    gfX = nStep > 0 ? fRatio : fHalf;
    goTarget = OBJECT_SELF;
    vHere.x = fFive;
    vZero = vFlat;
}

void main()
{
    ReportProgress("start");
}
