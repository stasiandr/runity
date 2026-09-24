//! Every number a module says goes from 0 to 1 is one of its value's: a
//! path that names nothing is a typo, and would leave a slider a box.

use scrap::shape::Shape;

fn fractions(shape: &Shape) -> usize {
    match shape {
        Shape::Fraction => 1,
        Shape::Option(s) | Shape::List(s) | Shape::Map(_, s) => fractions(s),
        Shape::Tuple(items) | Shape::OneOf(items) => items.iter().map(fractions).sum(),
        Shape::Struct(fields) | Shape::Tagged(fields) => {
            fields.iter().map(|(_, s)| fractions(s)).sum()
        }
        _ => 0,
    }
}

#[test]
fn every_fraction_a_module_declares_is_a_number_of_its_value() {
    // `with_fractions` panics on a path that names nothing (debug builds).
    let shapes: Vec<(&str, Shape)> = scrap::scene::part_kinds()
        .into_iter()
        .map(|k| (k.name, (k.shape)()))
        .collect();
    let count = |name: &str| fractions(&shapes.iter().find(|(n, _)| *n == name).unwrap().1);
    assert_eq!(count("weather"), 12);
    assert_eq!(count("post"), scrap::post::FRACTIONS.len());
    assert_eq!(count("post_volume"), scrap::post::FRACTIONS.len());
    assert_eq!(count("material"), scrap::material::FRACTIONS.len());
    assert_eq!(count("light"), 0, "a light's intensity goes past 1");
}
