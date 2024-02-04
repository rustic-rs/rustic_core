use std::collections::HashMap;

/// Extend the contents of a [`HashMap`] with the contents of another
/// [`HashMap`] with the same key and value types.
fn extend(left: &mut HashMap<String, String>, right: HashMap<String, String>) {
    left.extend(right);
}
