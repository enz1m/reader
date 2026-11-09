use super::*;

#[test]
fn argb32_multi_row_uses_stride_per_row() {
    let data = vec![
        0, 0, 255, 255, 9, 9, 9, 9, // row 0 + padding
        255, 0, 0, 255, 7, 7, 7, 7, // row 1 + padding
    ];
    let out = argb32_to_rgba(&data, 8, 1, 2);
    assert_eq!(
        out,
        vec![
            255, 0, 0, 255, /* row0 red */ 0, 0, 255, 255 /* row1 blue */
        ],
        "each row reordered BGRA→RGBA, padding dropped"
    );
}
