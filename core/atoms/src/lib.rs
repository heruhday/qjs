use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default, PartialOrd, Ord)]
pub struct Atom(pub u32);

#[derive(Debug, Clone)]
pub struct AtomEntry {
    pub(crate) text: String,
    pub(crate) marked: bool,
}

#[derive(Debug, Default)]
pub struct AtomTable {
    map: HashMap<String, Atom>,
    vec: Vec<Option<AtomEntry>>,
    free: Vec<u32>,
}

impl AtomTable {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn intern(&mut self, s: &str) -> Atom {
        if let Some(atom) = self.map.get(s) {
            return *atom;
        }

        let atom = if let Some(index) = self.free.pop() {
            self.vec[index as usize] = Some(AtomEntry {
                text: s.to_owned(),
                marked: false,
            });
            Atom(index)
        } else {
            let atom = Atom(self.vec.len() as u32);
            self.vec.push(Some(AtomEntry {
                text: s.to_owned(),
                marked: false,
            }));
            atom
        };

        self.map.insert(s.to_owned(), atom);
        atom
    }

    pub fn resolve(&self, atom: Atom) -> &str {
        self.vec
            .get(atom.0 as usize)
            .and_then(Option::as_ref)
            .map(|entry| entry.text.as_str())
            .expect("attempted to resolve an invalid atom")
    }

    pub fn mark(&mut self, atom: Atom) {
        if let Some(Some(entry)) = self.vec.get_mut(atom.0 as usize) {
            entry.marked = true;
        }
    }

    pub fn sweep(&mut self) {
        for (index, slot) in self.vec.iter_mut().enumerate() {
            if let Some(entry) = slot {
                if entry.marked {
                    entry.marked = false;
                } else {
                    self.map.remove(&entry.text);
                    *slot = None;
                    self.free.push(index as u32);
                }
            }
        }
    }

    pub fn count(&self) -> usize {
        self.map.len()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PropFlags(u8);

impl PropFlags {
    pub const WRITABLE: Self = Self(1);
    pub const ENUMERABLE: Self = Self(2);
    pub const CONFIGURABLE: Self = Self(4);
}

#[derive(Debug, Default)]
pub struct Shape {
    pub props: HashMap<Atom, usize>,
    pub flags: HashMap<Atom, PropFlags>,
    pub transitions: RefCell<HashMap<Atom, Rc<Shape>>>,
    #[allow(dead_code)]
    sorted_props: RefCell<Option<Vec<(Atom, usize)>>>,
}

impl Shape {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn transition(&self, atom: Atom) -> Rc<Shape> {
        if let Some(shape) = self.transitions.borrow().get(&atom) {
            return shape.clone();
        }

        let mut props = self.props.clone();
        let mut flags = self.flags.clone();
        let idx = props.len();

        props.insert(atom, idx);
        flags.insert(atom, PropFlags::WRITABLE);

        let next = Rc::new(Shape {
            props,
            flags,
            transitions: RefCell::new(HashMap::new()),
            sorted_props: RefCell::new(None),
        });

        self.transitions.borrow_mut().insert(atom, next.clone());
        next
    }

    pub fn trace_atoms(&self, atoms: &mut AtomTable) {
        for atom in self.props.keys().copied() {
            atoms.mark(atom);
        }

        for atom in self.flags.keys().copied() {
            atoms.mark(atom);
        }
    }

    pub fn sorted_props(&self) -> std::cell::Ref<'_, Vec<(Atom, usize)>> {
        if self.sorted_props.borrow().is_none() {
            let mut entries: Vec<(Atom, usize)> = self
                .props
                .iter()
                .map(|(&atom, &index)| (atom, index))
                .collect();
            entries.sort_by_key(|(atom, _)| atom.0);
            *self.sorted_props.borrow_mut() = Some(entries);
        }

        std::cell::Ref::map(self.sorted_props.borrow(), |entries| {
            entries
                .as_ref()
                .expect("sorted_props cache must be initialized before borrow mapping")
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interns_and_resolves_atoms() {
        let mut table = AtomTable::new();

        let first = table.intern("hello");
        let second = table.intern("hello");

        assert_eq!(first, second);
        assert_eq!(table.resolve(first), "hello");
        assert_eq!(table.count(), 1);
    }

    #[test]
    fn shape_transition_adds_property() {
        let atom = Atom(1);
        let shape = Shape::new();
        let next = shape.transition(atom);

        assert_eq!(next.props.get(&atom), Some(&0));
    }
}
