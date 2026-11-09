//! Transactions record object-level changes so they can be undone, redone and reported to
//! consumers (connectivity, render scenes, MCP clients).

use model::{BoardLayer, DesignRules, Object, ObjectId, Project};

use crate::CommandError;

/// One object's change: inserted (`before == None`), removed (`after == None`) or updated.
/// `before` keeps the object's index in its collection so undo restores the exact order.
#[derive(Clone, Debug, PartialEq)]
pub struct Change {
    pub id: ObjectId,
    pub before: Option<(usize, Object)>,
    pub after: Option<Object>,
}

impl Change {
    pub fn is_insert(&self) -> bool {
        self.before.is_none() && self.after.is_some()
    }

    pub fn is_remove(&self) -> bool {
        self.before.is_some() && self.after.is_none()
    }

    pub fn before_object(&self) -> Option<&Object> {
        self.before.as_ref().map(|(_, o)| o)
    }
}

/// The board's layer stack and design rules. Not objects: changed as a whole.
#[derive(Clone, Debug, PartialEq)]
pub struct BoardSetup {
    pub layers: Vec<BoardLayer>,
    pub rules: DesignRules,
}

impl BoardSetup {
    pub fn of(project: &Project) -> BoardSetup {
        BoardSetup { layers: project.board.layers.clone(), rules: project.board.rules.clone() }
    }

    fn apply(&self, project: &mut Project) {
        project.board.layers = self.layers.clone();
        project.board.rules = self.rules.clone();
    }
}

/// Changes of one command (or one undo/redo step), one entry per object.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ChangeSet {
    pub changes: Vec<Change>,
    /// Board setup before and after, if it changed.
    pub setup: Option<(BoardSetup, BoardSetup)>,
}

impl ChangeSet {
    pub fn is_empty(&self) -> bool {
        self.changes.is_empty() && self.setup.is_none()
    }

    pub fn ids(&self) -> impl Iterator<Item = ObjectId> + '_ {
        self.changes.iter().map(|c| c.id)
    }

    pub fn touches_schematic(&self) -> bool {
        self.ids().any(|id| id.kind().is_schematic())
    }

    pub fn touches_board(&self) -> bool {
        self.setup.is_some() || self.ids().any(|id| id.kind().is_board())
    }

    /// `self` followed by `next`, as one change set.
    pub fn then(&self, next: ChangeSet) -> ChangeSet {
        let mut all = self.changes.clone();
        all.extend(next.changes);
        let setup = match (&self.setup, next.setup) {
            (Some((before, _)), Some((_, after))) => Some((before.clone(), after)),
            (Some(s), None) => Some(s.clone()),
            (None, s) => s,
        };
        ChangeSet::coalesce(all, setup)
    }

    /// Merge several changes of the same object into one (first `before`, last `after`),
    /// keeping first-occurrence order, and drop changes without net effect.
    fn coalesce(changes: Vec<Change>, setup: Option<(BoardSetup, BoardSetup)>) -> ChangeSet {
        let mut out: Vec<Change> = Vec::with_capacity(changes.len());
        let mut pos: std::collections::HashMap<ObjectId, usize> = Default::default();
        for c in changes {
            match pos.get(&c.id) {
                Some(&i) => out[i].after = c.after,
                None => {
                    pos.insert(c.id, out.len());
                    out.push(c);
                }
            }
        }
        out.retain(|c| match (&c.before, &c.after) {
            (None, None) => false,
            (Some((_, b)), Some(a)) => b != a,
            _ => true,
        });
        ChangeSet { changes: out, setup: setup.filter(|(b, a)| b != a) }
    }
}

/// Undo `set`: apply its changes backwards. Returns the effective changes (for consumers).
pub(crate) fn apply_reverse(project: &mut Project, set: &ChangeSet) -> ChangeSet {
    let mut done = Vec::with_capacity(set.changes.len());
    for c in set.changes.iter().rev() {
        match (&c.before, &c.after) {
            (None, Some(_)) => {
                let taken = project.take(c.id).expect("undo: inserted object exists");
                done.push(Change { id: c.id, before: Some(taken), after: None });
            }
            (Some((index, old)), None) => {
                project.put(old.clone(), Some(*index));
                done.push(Change { id: c.id, before: None, after: Some(old.clone()) });
            }
            (Some((_, old)), Some(_)) => {
                let (index, current) = project.replace(old.clone()).expect("undo: updated object exists");
                done.push(Change { id: c.id, before: Some((index, current)), after: Some(old.clone()) });
            }
            (None, None) => {}
        }
    }
    if let Some((before, after)) = &set.setup {
        before.apply(project);
        return ChangeSet { changes: done, setup: Some((after.clone(), before.clone())) };
    }
    ChangeSet { changes: done, setup: None }
}

/// Redo `set`: apply its changes forwards.
pub(crate) fn apply_forward(project: &mut Project, set: &ChangeSet) -> ChangeSet {
    let mut done = Vec::with_capacity(set.changes.len());
    for c in &set.changes {
        match (&c.before, &c.after) {
            (None, Some(new)) => {
                project.put(new.clone(), None);
                done.push(c.clone());
            }
            (Some(_), None) => {
                let taken = project.take(c.id).expect("redo: removed object exists");
                done.push(Change { id: c.id, before: Some(taken), after: None });
            }
            (Some(_), Some(new)) => {
                let (index, current) = project.replace(new.clone()).expect("redo: updated object exists");
                done.push(Change { id: c.id, before: Some((index, current)), after: Some(new.clone()) });
            }
            (None, None) => {}
        }
    }
    if let Some(setup) = &set.setup {
        setup.1.apply(project);
    }
    ChangeSet { changes: done, setup: set.setup.clone() }
}

/// Mutable access to a project that records every change.
pub struct Transaction<'p> {
    project: &'p mut Project,
    changes: Vec<Change>,
    /// Board setup before the first [`Transaction::set_setup`].
    setup_before: Option<BoardSetup>,
}

impl<'p> Transaction<'p> {
    pub fn new(project: &'p mut Project) -> Self {
        Self { project, changes: Vec::new(), setup_before: None }
    }

    pub fn project(&self) -> &Project {
        self.project
    }

    /// Changes recorded so far (not coalesced).
    pub fn changes(&self) -> &[Change] {
        &self.changes
    }

    pub fn insert(&mut self, object: Object) -> Result<ObjectId, CommandError> {
        let id = object.id();
        if self.project.contains(id) {
            return Err(CommandError::Duplicate(id));
        }
        self.project.put(object.clone(), None);
        self.changes.push(Change { id, before: None, after: Some(object) });
        Ok(id)
    }

    pub fn remove(&mut self, id: ObjectId) -> Result<Object, CommandError> {
        let (index, object) = self.project.take(id).ok_or(CommandError::NotFound(id))?;
        self.changes.push(Change { id, before: Some((index, object.clone())), after: None });
        Ok(object)
    }

    /// Modify an object in place. The closure must not change the object's id.
    pub fn update(
        &mut self,
        id: ObjectId,
        f: impl FnOnce(&mut Object) -> Result<(), CommandError>,
    ) -> Result<(), CommandError> {
        let mut object = self.project.get(id).ok_or(CommandError::NotFound(id))?;
        f(&mut object)?;
        if object.id() != id {
            return Err(CommandError::Invalid("an update must not change the object id".into()));
        }
        self.replace(object)
    }

    /// Replace an existing object (matched by id), keeping its position.
    pub fn replace(&mut self, object: Object) -> Result<(), CommandError> {
        let id = object.id();
        let (index, old) = self.project.replace(object.clone()).ok_or(CommandError::NotFound(id))?;
        self.changes.push(Change { id, before: Some((index, old)), after: Some(object) });
        Ok(())
    }

    /// Change the layer stack and design rules.
    pub fn set_setup(&mut self, setup: BoardSetup) {
        if self.setup_before.is_none() {
            self.setup_before = Some(BoardSetup::of(self.project));
        }
        setup.apply(self.project);
    }

    /// Undo everything recorded so far.
    pub fn rollback(self) {
        if let Some(before) = &self.setup_before {
            before.apply(self.project);
        }
        apply_reverse(self.project, &ChangeSet { changes: self.changes, setup: None });
    }

    pub fn finish(self) -> ChangeSet {
        let setup = self.setup_before.map(|b| (b, BoardSetup::of(self.project)));
        ChangeSet::coalesce(self.changes, setup)
    }
}
