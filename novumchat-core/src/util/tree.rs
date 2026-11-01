//! Conversation trees.
//!
//! Messages form a tree through `parent_id`: regenerating or editing creates a
//! sibling, an ensemble request creates several sibling responses that share a
//! `group_id`. A chat shows one branch at a time: the path from the root to the
//! chat's `current_leaf`.

use std::collections::HashMap;

use crate::types::{Message, Role};

pub struct Tree<'a> {
    by_id: HashMap<&'a str, &'a Message>,
    children: HashMap<Option<&'a str>, Vec<&'a Message>>,
    newest: Option<&'a Message>,
}

impl<'a> Tree<'a> {
    pub fn new(messages: &'a [Message]) -> Self {
        let mut by_id = HashMap::with_capacity(messages.len());
        let mut children: HashMap<Option<&'a str>, Vec<&'a Message>> = HashMap::new();
        let mut newest: Option<&'a Message> = None;
        for m in messages {
            by_id.insert(m.id.as_str(), m);
            if newest.is_none_or(|n| (m.created_at, &m.id) >= (n.created_at, &n.id)) {
                newest = Some(m);
            }
        }
        for m in messages {
            // Orphans (parent deleted) are treated as roots.
            let parent = m.parent_id.as_deref().filter(|p| by_id.contains_key(p));
            children.entry(parent).or_default().push(m);
        }
        for list in children.values_mut() {
            list.sort_by(|a, b| (a.created_at, &a.id).cmp(&(b.created_at, &b.id)));
        }
        Self { by_id, children, newest }
    }

    pub fn get(&self, id: &str) -> Option<&'a Message> {
        self.by_id.get(id).copied()
    }

    pub fn len(&self) -> usize {
        self.by_id.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_id.is_empty()
    }

    fn parent_key(&self, m: &Message) -> Option<&'a str> {
        m.parent_id.as_deref().and_then(|p| self.by_id.get_key_value(p).map(|(k, _)| *k))
    }

    pub fn children_of(&self, parent: Option<&str>) -> &[&'a Message] {
        let key = parent.and_then(|p| self.by_id.get_key_value(p).map(|(k, _)| *k));
        self.children.get(&key).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Root → `leaf` (inclusive). Empty if `leaf` is unknown.
    pub fn path_to(&self, leaf: &str) -> Vec<&'a Message> {
        let mut path = Vec::new();
        let mut cur = self.get(leaf);
        let mut guard = 0;
        while let Some(m) = cur {
            path.push(m);
            guard += 1;
            if guard > self.by_id.len() {
                break; // cycle protection
            }
            cur = self.parent_key(m).and_then(|p| self.get(p));
        }
        path.reverse();
        path
    }

    /// Preferred child when descending: a synthesis answer wins inside its
    /// group, otherwise the newest child.
    fn preferred_child(&self, id: &str) -> Option<&'a Message> {
        let kids = self.children_of(Some(id));
        kids.iter().rev().find(|m| m.synthesis).or_else(|| kids.last()).copied()
    }

    /// Follows preferred children down from `id`.
    pub fn deepest_leaf(&self, id: &str) -> Option<&'a str> {
        let mut cur = self.get(id)?;
        let mut guard = 0;
        while let Some(next) = self.preferred_child(&cur.id) {
            cur = next;
            guard += 1;
            if guard > self.by_id.len() {
                break;
            }
        }
        Some(cur.id.as_str())
    }

    /// The leaf to display when the chat has none (or an invalid one).
    pub fn default_leaf(&self) -> Option<&'a str> {
        let newest = self.newest?;
        self.deepest_leaf(&newest.id)
    }

    /// The displayed branch.
    pub fn active_path(&self, current_leaf: Option<&str>) -> Vec<&'a Message> {
        match current_leaf.filter(|l| self.by_id.contains_key(l)) {
            Some(leaf) => self.path_to(leaf),
            None => self.default_leaf().map(|l| self.path_to(l)).unwrap_or_default(),
        }
    }

    /// Ensemble siblings of `m` (same parent and group), synthesis answers first.
    pub fn group_members(&self, m: &Message) -> Vec<&'a Message> {
        let Some(group) = m.group_id.as_deref() else {
            return self.get(&m.id).into_iter().collect();
        };
        let mut members: Vec<&'a Message> = self
            .children_of(m.parent_id.as_deref())
            .iter()
            .copied()
            .filter(|s| s.group_id.as_deref() == Some(group))
            .collect();
        members.sort_by_key(|s| !s.synthesis);
        members
    }

    /// Alternatives for `m`: siblings with the same role grouped into units (a
    /// whole ensemble group is one unit). Returns the units and the index of
    /// the unit containing `m`.
    pub fn sibling_units(&self, m: &Message) -> (Vec<Vec<&'a Message>>, usize) {
        let mut units: Vec<Vec<&'a Message>> = Vec::new();
        let mut group_index: HashMap<&str, usize> = HashMap::new();
        for s in self.children_of(m.parent_id.as_deref()) {
            if s.role != m.role {
                continue;
            }
            match s.group_id.as_deref() {
                Some(g) => {
                    if let Some(&i) = group_index.get(g) {
                        units[i].push(s);
                    } else {
                        group_index.insert(g, units.len());
                        units.push(vec![s]);
                    }
                }
                None => units.push(vec![s]),
            }
        }
        let idx = units.iter().position(|u| u.iter().any(|s| s.id == m.id)).unwrap_or(0);
        (units, idx)
    }

    /// Ids of `id` and all of its descendants.
    pub fn descendants(&self, id: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut stack = vec![id.to_string()];
        while let Some(cur) = stack.pop() {
            if out.contains(&cur) {
                continue;
            }
            for c in self.children_of(Some(&cur)) {
                stack.push(c.id.clone());
            }
            out.push(cur);
        }
        out
    }

    /// The user message an assistant message answers (walks up to the nearest user).
    pub fn prompt_for(&self, m: &Message) -> Option<&'a Message> {
        let mut cur = self.parent_key(m).and_then(|p| self.get(p));
        while let Some(c) = cur {
            if c.role == Role::User {
                return Some(c);
            }
            cur = self.parent_key(c).and_then(|p| self.get(p));
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{MessageStatus, Part};

    fn msg(id: &str, parent: Option<&str>, role: Role, t: i64, group: Option<&str>) -> Message {
        Message {
            id: id.into(),
            chat_id: "c".into(),
            parent_id: parent.map(Into::into),
            role,
            parts: vec![Part::Text { text: id.into() }],
            attachments: vec![],
            model: None,
            group_id: group.map(Into::into),
            synthesis: false,
            status: MessageStatus::Complete,
            error: None,
            usage: None,
            created_at: t,
            duration_ms: None,
            ttft_ms: None,
        }
    }

    #[test]
    fn branches_and_groups() {
        let mut msgs = vec![
            msg("u1", None, Role::User, 1, None),
            msg("a1", Some("u1"), Role::Assistant, 2, Some("g")),
            msg("a2", Some("u1"), Role::Assistant, 3, Some("g")),
            msg("u2", Some("a1"), Role::User, 4, None),
            msg("a3", Some("u2"), Role::Assistant, 5, None),
            msg("r1", Some("u1"), Role::Assistant, 6, None),
        ];
        let tree = Tree::new(&msgs);
        // Newest message is r1 → default branch u1 → r1.
        let ids: Vec<_> = tree.active_path(None).iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, ["u1", "r1"]);
        // Explicit leaf.
        let ids: Vec<_> = tree.active_path(Some("a3")).iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, ["u1", "a1", "u2", "a3"]);
        // Units: group g (a1, a2) and r1.
        let a1 = tree.get("a1").unwrap();
        let (units, idx) = tree.sibling_units(a1);
        assert_eq!(units.len(), 2);
        assert_eq!(units[0].len(), 2);
        assert_eq!(idx, 0);
        assert_eq!(tree.group_members(a1).len(), 2);
        assert_eq!(tree.deepest_leaf("a1"), Some("a3"));
        assert_eq!(tree.prompt_for(tree.get("a3").unwrap()).unwrap().id, "u2");
        let mut d = tree.descendants("a1");
        d.sort();
        assert_eq!(d, ["a1", "a3", "u2"]);

        // A synthesis answer is preferred when descending.
        let mut s = msg("s", Some("u1"), Role::Assistant, 7, Some("g"));
        s.synthesis = true;
        msgs.push(s);
        let tree = Tree::new(&msgs);
        assert_eq!(tree.deepest_leaf("u1"), Some("s"));
        assert_eq!(tree.group_members(tree.get("a2").unwrap())[0].id, "s");
    }
}
