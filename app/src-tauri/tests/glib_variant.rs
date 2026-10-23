#![cfg(target_os = "linux")]

#[test]
fn optimized_string_variant_iterators() {
    use glib::variant::ToVariant;
    let value = ["first", "", "中文", "last"].to_variant();
    assert_eq!(
        value.array_iter_str().unwrap().collect::<Vec<_>>(),
        ["first", "", "中文", "last"]
    );
    assert_eq!(value.array_iter_str().unwrap().nth(2), Some("中文"));
    assert_eq!(value.array_iter_str().unwrap().last(), Some("last"));
    assert_eq!(value.array_iter_str().unwrap().next_back(), Some("last"));
    assert_eq!(value.array_iter_str().unwrap().nth_back(2), Some(""));
    let mut iterator = value.array_iter_str().unwrap();
    assert_eq!(iterator.next(), Some("first"));
    assert_eq!(iterator.next_back(), Some("last"));
    assert_eq!(iterator.next(), Some(""));
    assert_eq!(iterator.next_back(), Some("中文"));
    assert_eq!(iterator.next(), None);
}
