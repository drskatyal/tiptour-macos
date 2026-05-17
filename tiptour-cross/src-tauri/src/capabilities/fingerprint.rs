// Structural hash of a UIA/AX tree snapshot. The grounding layer hands us
// an opaque snapshot — we walk it and hash (role, name, parent-path) tuples
// so cosmetic re-renders that don't change structure produce the same id.
//
// On platforms without an accessibility backend wired in, fingerprint() just
// rolls the snapshot's stringified payload into a stable hash so the
// explorer can still make forward progress in unit tests.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

#[derive(Debug, Clone)]
pub struct TreeNodeSnapshot {
    pub role: String,
    pub name: String,
    pub children: Vec<TreeNodeSnapshot>,
}

#[derive(Debug, Clone)]
pub struct TreeSnapshot {
    pub foreground_window_title: Option<String>,
    pub root: TreeNodeSnapshot,
}

pub fn fingerprint(snapshot: &TreeSnapshot) -> String {
    let mut hasher = DefaultHasher::new();
    if let Some(title) = &snapshot.foreground_window_title {
        // Window title is part of the fingerprint because two different
        // documents in the same app should be different states; without it,
        // "Untitled - Word" and "Resume.docx - Word" would collide.
        title.hash(&mut hasher);
    }
    hash_node(&snapshot.root, &[], &mut hasher);
    format!("{:016x}", hasher.finish())
}

fn hash_node(node: &TreeNodeSnapshot, parent_path: &[&str], hasher: &mut DefaultHasher) {
    node.role.hash(hasher);
    node.name.hash(hasher);
    for ancestor in parent_path {
        ancestor.hash(hasher);
    }

    let mut child_path: Vec<&str> = parent_path.to_vec();
    child_path.push(node.name.as_str());
    for child in &node.children {
        hash_node(child, &child_path, hasher);
    }
}

pub fn element_count(snapshot: &TreeSnapshot) -> usize {
    fn walk(node: &TreeNodeSnapshot) -> usize {
        1 + node.children.iter().map(walk).sum::<usize>()
    }
    walk(&snapshot.root)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leaf(role: &str, name: &str) -> TreeNodeSnapshot {
        TreeNodeSnapshot {
            role: role.into(),
            name: name.into(),
            children: vec![],
        }
    }

    #[test]
    fn same_structure_same_fingerprint() {
        let a = TreeSnapshot {
            foreground_window_title: Some("App".into()),
            root: TreeNodeSnapshot {
                role: "Window".into(),
                name: "Main".into(),
                children: vec![leaf("Button", "OK"), leaf("Button", "Cancel")],
            },
        };
        let b = a.clone();
        assert_eq!(fingerprint(&a), fingerprint(&b));
    }

    #[test]
    fn different_structure_differs() {
        let a = TreeSnapshot {
            foreground_window_title: Some("App".into()),
            root: TreeNodeSnapshot {
                role: "Window".into(),
                name: "Main".into(),
                children: vec![leaf("Button", "OK")],
            },
        };
        let b = TreeSnapshot {
            foreground_window_title: Some("App".into()),
            root: TreeNodeSnapshot {
                role: "Window".into(),
                name: "Main".into(),
                children: vec![leaf("Button", "Submit")],
            },
        };
        assert_ne!(fingerprint(&a), fingerprint(&b));
    }
}
