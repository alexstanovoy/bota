//! Ordered camp compositions by size and spawn type.
//! These rosters are held until maps use the matching spawn ranges and all roster kinds can be offered.

use crate::{NEUTRALS, NeutralKind};

/// Each size's rows in spawn type order, including flooded rows and empty rows.
pub const PATCH_CAMP_ROSTERS: [&[&[NeutralKind]]; 4] = [
    &[
        &[
            NEUTRALS[0].kind,
            NEUTRALS[0].kind,
            NEUTRALS[1].kind,
            NEUTRALS[2].kind,
        ],
        &[NEUTRALS[3].kind, NEUTRALS[3].kind, NEUTRALS[2].kind],
        &[NEUTRALS[3].kind, NEUTRALS[3].kind, NEUTRALS[4].kind],
        &[NEUTRALS[5].kind, NEUTRALS[5].kind, NEUTRALS[5].kind],
        &[NEUTRALS[6].kind, NEUTRALS[6].kind, NEUTRALS[7].kind],
        &[NEUTRALS[8].kind, NEUTRALS[8].kind, NEUTRALS[9].kind],
        &[NEUTRALS[38].kind, NEUTRALS[38].kind, NEUTRALS[38].kind],
    ],
    &[
        &[NEUTRALS[17].kind, NEUTRALS[17].kind, NEUTRALS[18].kind],
        &[NEUTRALS[12].kind, NEUTRALS[12].kind, NEUTRALS[13].kind],
        &[NEUTRALS[48].kind, NEUTRALS[48].kind],
        &[
            NEUTRALS[14].kind,
            NEUTRALS[14].kind,
            NEUTRALS[15].kind,
            NEUTRALS[15].kind,
        ],
        &[NEUTRALS[19].kind, NEUTRALS[19].kind],
        &[NEUTRALS[49].kind, NEUTRALS[49].kind, NEUTRALS[50].kind],
        &[NEUTRALS[42].kind, NEUTRALS[39].kind, NEUTRALS[39].kind],
    ],
    &[
        &[NEUTRALS[10].kind, NEUTRALS[11].kind, NEUTRALS[10].kind],
        &[NEUTRALS[52].kind, NEUTRALS[51].kind],
        &[NEUTRALS[23].kind, NEUTRALS[23].kind, NEUTRALS[24].kind],
        &[NEUTRALS[21].kind, NEUTRALS[22].kind],
        &[NEUTRALS[15].kind, NEUTRALS[15].kind, NEUTRALS[16].kind],
        &[NEUTRALS[25].kind, NEUTRALS[25].kind, NEUTRALS[26].kind],
        &[NEUTRALS[43].kind, NEUTRALS[40].kind, NEUTRALS[40].kind],
    ],
    &[
        &[NEUTRALS[30].kind, NEUTRALS[30].kind, NEUTRALS[31].kind],
        &[NEUTRALS[54].kind, NEUTRALS[54].kind, NEUTRALS[53].kind],
        &[NEUTRALS[34].kind, NEUTRALS[34].kind, NEUTRALS[35].kind],
        &[NEUTRALS[28].kind, NEUTRALS[28].kind, NEUTRALS[29].kind],
        &[NEUTRALS[36].kind, NEUTRALS[36].kind, NEUTRALS[37].kind],
        &[],
        &[NEUTRALS[44].kind, NEUTRALS[41].kind, NEUTRALS[41].kind],
    ],
];
