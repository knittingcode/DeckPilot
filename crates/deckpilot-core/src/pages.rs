//! The page tree of a layout: folders, order, Back navigation, and moving
//! or copying pages between layouts.
//!
//! Every page except the home page may belong to a folder
//! ([`LayoutPage::parent`]); Back returns to it. Layouts written before
//! parents were stored have none: their tree is **inferred** when read —
//! a page's folder is the page that opens it first, in a breadth-first
//! walk from the home page through `openPage` bindings (pages in layout
//! order, bindings in page order). Explicit parents always win and
//! inference never creates a cycle. Inferring changes nothing stored, so
//! a layout saved without edits stays byte for byte the same; the first
//! structural edit (reordering, refiling, moving or copying a page) stores
//! the tree explicitly and sets [`DeviceLayout::explicit_tree`]
//! ([`DeviceLayout::infer_parents`], lossless and idempotent). From then
//! on nothing is inferred and a page without a parent is top level.
//!
//! **Back.** A page inside a folder must have a way back: a Back binding
//! or a binding that opens one of its folders. Pages without one are a
//! warning ([`LayoutIssue::MissingBack`]), not an error, so older layouts
//! keep loading and importing; the runtime then shows an automatic Back
//! key on the page's first free key ([`DeviceLayout::automatic_back`]),
//! never on a bound one.

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};

use deckpilot_device::{ControlId, ControlKind, ModelCatalog};

use crate::{
    BindingTarget, DeviceLayout, LayoutId, LayoutIssue, LayoutPage, PageId, Profile, ProfileIssue,
};

impl DeviceLayout {
    /// Whether making `parent` the folder of `page` keeps the tree acyclic,
    /// given the parents in `parents`.
    fn creates_cycle(parents: &BTreeMap<PageId, PageId>, page: PageId, parent: PageId) -> bool {
        let mut current = Some(parent);
        let mut steps = 0;
        while let Some(id) = current {
            if id == page || steps > parents.len() {
                return true;
            }
            current = parents.get(&id).copied();
            steps += 1;
        }
        false
    }

    /// The folder of every page that is in one: its explicit parent, or
    /// the inferred one (see the module documentation). Invalid explicit
    /// parents (unknown, cyclic, on the home page) are left out.
    pub fn page_parents(&self) -> BTreeMap<PageId, PageId> {
        let exists: BTreeSet<PageId> = self.pages.iter().map(|p| p.id).collect();
        let mut parents = BTreeMap::new();
        for page in &self.pages {
            if let Some(parent) = page.parent
                && page.id != self.home_page
                && exists.contains(&parent)
                && !Self::creates_cycle(&parents, page.id, parent)
            {
                parents.insert(page.id, parent);
            }
        }
        if self.explicit_tree {
            return parents;
        }
        let explicit: BTreeSet<PageId> = parents.keys().copied().collect();
        let mut visited = BTreeSet::from([self.home_page]);
        let mut queue = VecDeque::from([self.home_page]);
        while let Some(current) = queue.pop_front() {
            let Some(page) = self.page(current) else {
                continue;
            };
            let opened = page.bindings.iter().filter_map(|b| match b.target {
                BindingTarget::OpenPage { page } => Some(page),
                _ => None,
            });
            // Pages explicitly inside this one are part of the walk too.
            let children = self
                .pages
                .iter()
                .filter(|p| parents.get(&p.id) == Some(&current) && explicit.contains(&p.id))
                .map(|p| p.id)
                .collect::<Vec<_>>();
            for target in opened.chain(children) {
                if !exists.contains(&target) || !visited.insert(target) {
                    continue;
                }
                if target != self.home_page
                    && !parents.contains_key(&target)
                    && !Self::creates_cycle(&parents, target, current)
                {
                    parents.insert(target, current);
                }
                queue.push_back(target);
            }
        }
        parents
    }

    /// The folder `page` belongs to.
    pub fn parent_of(&self, page: PageId) -> Option<PageId> {
        self.page_parents().get(&page).copied()
    }

    /// The folders `page` is in, the nearest first.
    pub fn ancestors(&self, page: PageId) -> Vec<PageId> {
        let parents = self.page_parents();
        let mut out = Vec::new();
        let mut current = parents.get(&page).copied();
        while let Some(id) = current {
            if out.contains(&id) {
                break;
            }
            out.push(id);
            current = parents.get(&id).copied();
        }
        out
    }

    /// `page` and every page inside it, at any depth, in layout order
    /// (`page` first).
    pub fn subtree(&self, page: PageId) -> Vec<PageId> {
        let parents = self.page_parents();
        let mut inside = BTreeSet::from([page]);
        // Parents may come after their children in the list: repeat until
        // nothing is added.
        loop {
            let before = inside.len();
            for (child, parent) in &parents {
                if inside.contains(parent) {
                    inside.insert(*child);
                }
            }
            if inside.len() == before {
                break;
            }
        }
        let mut out = vec![page];
        out.extend(
            self.pages
                .iter()
                .map(|p| p.id)
                .filter(|id| *id != page && inside.contains(id)),
        );
        out
    }

    /// Stores the current tree (explicit and inferred parents) explicitly
    /// and marks it as stored ([`DeviceLayout::explicit_tree`]). Returns how
    /// many parents were added. Lossless (bindings and existing parents are
    /// untouched, the tree stays the same) and idempotent (a second call
    /// changes nothing).
    pub fn infer_parents(&mut self) -> usize {
        let parents = self.page_parents();
        self.explicit_tree = true;
        let mut added = 0;
        for page in &mut self.pages {
            if page.parent.is_none()
                && let Some(parent) = parents.get(&page.id)
            {
                page.parent = Some(*parent);
                added += 1;
            }
        }
        added
    }

    /// Puts `page` into the folder `parent` (`None`: top level). The tree is
    /// stored explicitly first. Refuses the home page and cycles.
    pub fn set_parent(&mut self, page: PageId, parent: Option<PageId>) -> Result<(), ProfileIssue> {
        if self.page(page).is_none() || parent.is_some_and(|p| self.page(p).is_none()) {
            return Err(ProfileIssue::NotFound("page"));
        }
        if page == self.home_page {
            return Err(ProfileIssue::HomePage);
        }
        if let Some(parent) = parent
            && self.subtree(page).contains(&parent)
        {
            return Err(ProfileIssue::PageCycle);
        }
        self.infer_parents();
        if let Some(entry) = self.page_mut(page) {
            entry.parent = parent;
        }
        Ok(())
    }

    /// Puts the pages in `order`, which must list every page exactly once.
    pub fn reorder_pages(&mut self, order: &[PageId]) -> Result<(), ProfileIssue> {
        let current: BTreeSet<PageId> = self.pages.iter().map(|p| p.id).collect();
        let requested: BTreeSet<PageId> = order.iter().copied().collect();
        if order.len() != self.pages.len() || requested != current {
            return Err(ProfileIssue::PageOrder);
        }
        // Inference follows the page order; keep the tree as it was.
        self.infer_parents();
        let mut pages: HashMap<PageId, LayoutPage> =
            self.pages.drain(..).map(|p| (p.id, p)).collect();
        self.pages = order.iter().filter_map(|id| pages.remove(id)).collect();
        Ok(())
    }

    /// Whether a page inside a folder has a way back: a Back binding, or a
    /// binding that opens one of its folders. Pages outside folders need
    /// none.
    pub fn has_way_back(&self, page: PageId) -> bool {
        let ancestors = self.ancestors(page);
        if ancestors.is_empty() {
            return true;
        }
        self.page(page).is_none_or(|p| {
            p.bindings.iter().any(|b| match b.target {
                BindingTarget::Back => true,
                BindingTarget::OpenPage { page } => ancestors.contains(&page),
                _ => false,
            })
        })
    }

    /// The key the runtime uses as an automatic Back on a page inside a
    /// folder that has no way back: the first key of `controls` the page
    /// does not bind. `None` when the page needs none or every key is
    /// bound.
    pub fn automatic_back(&self, page: PageId, controls: &[ControlId]) -> Option<ControlId> {
        if self.has_way_back(page) {
            return None;
        }
        let bound: BTreeSet<ControlId> = self
            .page(page)?
            .bindings
            .iter()
            .map(|b| b.control)
            .collect();
        controls
            .iter()
            .copied()
            .find(|c| c.kind() == ControlKind::Key && !bound.contains(c))
    }

    /// Problems that do not make the layout invalid: pages inside folders
    /// without a way back.
    pub fn warnings(&self) -> Vec<LayoutIssue> {
        self.pages
            .iter()
            .filter(|page| !self.has_way_back(page.id))
            .map(|page| LayoutIssue::MissingBack {
                page: page.name.clone(),
            })
            .collect()
    }

    /// Explicit parents must exist, form no cycle and leave the home page
    /// at the top.
    pub(crate) fn validate_parents(&self, issues: &mut Vec<LayoutIssue>) {
        let explicit: BTreeMap<PageId, PageId> = self
            .pages
            .iter()
            .filter_map(|p| p.parent.map(|parent| (p.id, parent)))
            .collect();
        for page in &self.pages {
            let Some(parent) = page.parent else {
                continue;
            };
            if page.id == self.home_page {
                issues.push(LayoutIssue::HomeHasParent);
            } else if self.page(parent).is_none() {
                issues.push(LayoutIssue::UnknownParent {
                    page: page.name.clone(),
                });
            } else {
                let mut seen = BTreeSet::from([page.id]);
                let mut current = Some(parent);
                while let Some(id) = current {
                    if !seen.insert(id) {
                        issues.push(LayoutIssue::ParentCycle {
                            page: page.name.clone(),
                        });
                        break;
                    }
                    current = explicit.get(&id).copied();
                }
            }
        }
    }
}

/// Where a page goes when it is moved or copied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageDestination {
    pub layout: LayoutId,
    /// The folder to put it in; `None`: the destination's home page for
    /// another layout, the top level for a move within a layout, the
    /// original's folder for a copy within a layout.
    pub parent: Option<PageId>,
}

impl Profile {
    fn layout_index(&self, id: LayoutId) -> Result<usize, ProfileIssue> {
        self.layouts
            .iter()
            .position(|l| l.id == id)
            .ok_or(ProfileIssue::NotFound("layout"))
    }

    /// The pages of `subtree` from `source`, with new identifiers when
    /// `fresh`, ready to be added to `target`: page links inside the
    /// subtree point to the copies; links outside it are kept within the
    /// same layout and refused for another one.
    fn detach(
        source: &DeviceLayout,
        subtree: &[PageId],
        fresh: bool,
        same_layout: bool,
        root_parent: Option<PageId>,
    ) -> Result<Vec<LayoutPage>, ProfileIssue> {
        let ids: HashMap<PageId, PageId> = subtree
            .iter()
            .map(|id| (*id, if fresh { PageId::new() } else { *id }))
            .collect();
        let parents = source.page_parents();
        let mut pages = Vec::new();
        for (index, id) in subtree.iter().enumerate() {
            let Some(page) = source.page(*id) else {
                return Err(ProfileIssue::NotFound("page"));
            };
            let mut copy = page.clone();
            copy.id = ids[id];
            copy.parent = if index == 0 {
                root_parent
            } else {
                parents.get(id).map(|p| ids.get(p).copied().unwrap_or(*p))
            };
            for binding in &mut copy.bindings {
                if let BindingTarget::OpenPage { page } = &mut binding.target {
                    match ids.get(page) {
                        Some(new) => *page = *new,
                        None if same_layout => {}
                        None => return Err(ProfileIssue::OpensOutside),
                    }
                }
            }
            pages.push(copy);
        }
        Ok(pages)
    }

    /// Adds `pages` to the layout at `index` and checks it against its
    /// model: problems the pages bring are refused.
    fn attach(
        &mut self,
        index: usize,
        pages: Vec<LayoutPage>,
        catalog: &ModelCatalog,
    ) -> Result<(), ProfileIssue> {
        let before: BTreeSet<String> = self.layouts[index]
            .validate(catalog, self)
            .iter()
            .map(ToString::to_string)
            .collect();
        let mut target = self.layouts[index].clone();
        target.infer_parents();
        target.pages.extend(pages);
        let added: Vec<String> = target
            .validate(catalog, self)
            .iter()
            .map(ToString::to_string)
            .filter(|issue| !before.contains(issue))
            .collect();
        if !added.is_empty() {
            return Err(ProfileIssue::IncompatiblePage(added.join("; ")));
        }
        self.layouts[index] = target;
        Ok(())
    }

    /// Copies a page with its bindings and every page inside it to
    /// `destination` (the same or another layout; another layout's model
    /// must support every binding). The copies get new identifiers.
    /// Returns the copy's id.
    pub fn copy_page(
        &mut self,
        layout: LayoutId,
        page: PageId,
        destination: PageDestination,
        catalog: &ModelCatalog,
    ) -> Result<PageId, ProfileIssue> {
        let source_index = self.layout_index(layout)?;
        let target_index = self.layout_index(destination.layout)?;
        let source = &self.layouts[source_index];
        let target = &self.layouts[target_index];
        if source.page(page).is_none() {
            return Err(ProfileIssue::NotFound("page"));
        }
        if destination.parent.is_some_and(|p| target.page(p).is_none()) {
            return Err(ProfileIssue::NotFound("page"));
        }
        let same = source_index == target_index;
        let root_parent = match destination.parent {
            Some(parent) => Some(parent),
            None if same => source.parent_of(page),
            None => Some(target.home_page),
        };
        let pages = Self::detach(source, &source.subtree(page), true, same, root_parent)?;
        let id = pages[0].id;
        self.attach(target_index, pages, catalog)?;
        Ok(id)
    }

    /// Moves a page with its bindings and every page inside it. Within a
    /// layout it only changes folders (identifiers stay). To another
    /// layout, the pages keep their identifiers, and keys of the old layout
    /// that opened them are removed. The home page cannot be moved.
    /// Returns how many keys were removed from the old layout.
    pub fn move_page(
        &mut self,
        layout: LayoutId,
        page: PageId,
        destination: PageDestination,
        catalog: &ModelCatalog,
    ) -> Result<usize, ProfileIssue> {
        let source_index = self.layout_index(layout)?;
        let target_index = self.layout_index(destination.layout)?;
        if self.layouts[source_index].home_page == page {
            return Err(ProfileIssue::HomePage);
        }
        if source_index == target_index {
            self.layouts[source_index].set_parent(page, destination.parent)?;
            return Ok(0);
        }
        let source = &self.layouts[source_index];
        let target = &self.layouts[target_index];
        if source.page(page).is_none() {
            return Err(ProfileIssue::NotFound("page"));
        }
        let parent = match destination.parent {
            Some(parent) if target.page(parent).is_none() => {
                return Err(ProfileIssue::NotFound("page"));
            }
            Some(parent) => parent,
            None => target.home_page,
        };
        let subtree = source.subtree(page);
        if target.pages.iter().any(|p| subtree.contains(&p.id)) {
            return Err(ProfileIssue::PageCycle);
        }
        let pages = Self::detach(source, &subtree, false, false, Some(parent))?;
        self.attach(target_index, pages, catalog)?;
        let source = &mut self.layouts[source_index];
        source.infer_parents();
        source.pages.retain(|p| !subtree.contains(&p.id));
        let mut removed = 0;
        for remaining in &mut source.pages {
            let before = remaining.bindings.len();
            remaining.bindings.retain(|b| {
                !matches!(b.target, BindingTarget::OpenPage { page } if subtree.contains(&page))
            });
            removed += before - remaining.bindings.len();
        }
        Ok(removed)
    }
}

#[cfg(test)]
mod tests;
