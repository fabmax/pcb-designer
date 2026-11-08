//! CPU side scene: render layers of instances, owned by items, with dirty tracking.
//!
//! Every drawable thing (a footprint, a wire, a ratsnest net, …) is an *item* identified by
//! an [`ItemId`] chosen by the caller. An item owns slot ranges in one or more layers.
//! Replacing an item reuses its slots when the new geometry fits, otherwise appends.
//! Freed slots become [`Instance::EMPTY`] and are compacted away once they pile up.
//! [`Scene::take_upload`] returns only the changed instance ranges for the GPU.

use rustc_hash::FxHashMap;
use smallvec::SmallVec;

use crate::primitive::Instance;

pub type ItemId = u64;
pub type LayerId = u16;

#[derive(Clone, Debug)]
pub struct LayerDesc {
    pub name: String,
    pub visible: bool,
    /// Hide the layer when zoomed out further than this (pixels per world unit).
    pub min_zoom: f32,
}

#[derive(Clone, Copy, Debug)]
struct Alloc {
    layer: LayerId,
    start: u32,
    /// Slot capacity (may be larger than the item's current instance count).
    len: u32,
}

struct Layer {
    desc: LayerDesc,
    instances: Vec<Instance>,
    /// Number of freed slots (EMPTY) not owned by any item.
    garbage: usize,
    /// Changed slot ranges since the last upload (unsorted, may overlap).
    dirty: Vec<(usize, usize)>,
    full: bool,
    gpu_capacity: usize,
}

impl Layer {
    fn mark_dirty(&mut self, start: usize, end: usize) {
        // Extend the last range when contiguous (common for appends and in-place rewrites).
        if let Some(last) = self.dirty.last_mut()
            && start <= last.1
            && end >= last.0
        {
            *last = (last.0.min(start), last.1.max(end));
            return;
        }
        self.dirty.push((start, end));
    }

    /// Sorted, merged dirty ranges. Ranges closer than `GAP` slots are joined, since one
    /// slightly larger copy is cheaper than many tiny ones.
    fn take_ranges(&mut self) -> Vec<(usize, usize)> {
        const GAP: usize = 64;
        let mut r = std::mem::take(&mut self.dirty);
        r.sort_unstable();
        let mut out: Vec<(usize, usize)> = Vec::with_capacity(r.len());
        for (s, e) in r {
            match out.last_mut() {
                Some(last) if s <= last.1 + GAP => last.1 = last.1.max(e),
                _ => out.push((s, e)),
            }
        }
        out
    }
}

#[derive(Default)]
pub struct Scene {
    layers: Vec<Layer>,
    items: FxHashMap<ItemId, SmallVec<[Alloc; 2]>>,
    scratch: Vec<(LayerId, Instance)>,
}

/// Data to bring the GPU copy of a scene up to date.
#[derive(Default, Debug)]
pub struct SceneUpload {
    pub layers: Vec<LayerUpload>,
}

#[derive(Debug)]
pub struct LayerUpload {
    /// Number of instances to draw.
    pub len: usize,
    pub data: LayerData,
}

#[derive(Debug)]
pub enum LayerData {
    Unchanged,
    /// Reallocate the GPU buffer with `capacity` instances and upload `instances`.
    Full {
        capacity: usize,
        instances: Vec<Instance>,
    },
    /// Overwrite the given ranges: `(offset, instances)`.
    Partial(Vec<(usize, Vec<Instance>)>),
}

impl SceneUpload {
    pub fn is_empty(&self) -> bool {
        self.layers.iter().all(|l| matches!(l.data, LayerData::Unchanged))
    }

    pub fn bytes(&self) -> usize {
        self.layers
            .iter()
            .map(|l| match &l.data {
                LayerData::Unchanged => 0,
                LayerData::Full { instances, .. } => instances.len() * size_of::<Instance>(),
                LayerData::Partial(ranges) => {
                    ranges.iter().map(|(_, i)| i.len()).sum::<usize>() * size_of::<Instance>()
                }
            })
            .sum()
    }
}

impl Scene {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_layer(&mut self, name: impl Into<String>) -> LayerId {
        self.layers.push(Layer {
            desc: LayerDesc { name: name.into(), visible: true, min_zoom: 0.0 },
            instances: Vec::new(),
            garbage: 0,
            dirty: Vec::new(),
            full: true,
            gpu_capacity: 0,
        });
        (self.layers.len() - 1) as LayerId
    }

    pub fn layer_count(&self) -> usize {
        self.layers.len()
    }

    pub fn layer(&self, id: LayerId) -> &LayerDesc {
        &self.layers[id as usize].desc
    }

    pub fn layer_mut(&mut self, id: LayerId) -> &mut LayerDesc {
        &mut self.layers[id as usize].desc
    }

    /// Which layers should be drawn at the given zoom.
    pub fn visible_layers(&self, zoom: f64) -> Vec<bool> {
        self.layers.iter().map(|l| l.desc.visible && zoom >= l.desc.min_zoom as f64).collect()
    }

    /// Number of live (non-empty) instances.
    pub fn instance_count(&self) -> usize {
        self.layers.iter().map(|l| l.instances.len() - l.garbage).sum()
    }

    pub fn slot_count(&self) -> usize {
        self.layers.iter().map(|l| l.instances.len()).sum()
    }

    pub fn item_count(&self) -> usize {
        self.items.len()
    }

    pub fn contains(&self, id: ItemId) -> bool {
        self.items.contains_key(&id)
    }

    /// Replace the geometry of item `id` (creating it if needed).
    pub fn set_item(&mut self, id: ItemId, prims: impl IntoIterator<Item = (LayerId, Instance)>) {
        let mut new = std::mem::take(&mut self.scratch);
        new.clear();
        new.extend(prims);
        new.sort_by_key(|(l, _)| *l);

        let mut old = self.items.remove(&id).unwrap_or_default();
        let mut allocs: SmallVec<[Alloc; 2]> = SmallVec::new();

        let mut i = 0;
        while i < new.len() {
            let layer_id = new[i].0;
            let mut j = i;
            while j < new.len() && new[j].0 == layer_id {
                j += 1;
            }
            let group = &new[i..j];
            let layer = &mut self.layers[layer_id as usize];

            let reuse = old.iter().position(|a| a.layer == layer_id && a.len as usize >= group.len());
            if let Some(pos) = reuse {
                let a = old.swap_remove(pos);
                let start = a.start as usize;
                for (k, (_, inst)) in group.iter().enumerate() {
                    layer.instances[start + k] = *inst;
                }
                for slot in &mut layer.instances[start + group.len()..start + a.len as usize] {
                    *slot = Instance::EMPTY;
                }
                layer.mark_dirty(start, start + a.len as usize);
                allocs.push(a);
            } else {
                let start = layer.instances.len();
                layer.instances.extend(group.iter().map(|(_, inst)| *inst));
                layer.mark_dirty(start, layer.instances.len());
                allocs.push(Alloc { layer: layer_id, start: start as u32, len: group.len() as u32 });
            }
            i = j;
        }

        for a in old {
            self.free(a);
        }
        if !allocs.is_empty() {
            self.items.insert(id, allocs);
        }
        self.scratch = new;
    }

    pub fn remove_item(&mut self, id: ItemId) {
        if let Some(allocs) = self.items.remove(&id) {
            for a in allocs {
                self.free(a);
            }
        }
    }

    pub fn clear(&mut self) {
        self.items.clear();
        for layer in &mut self.layers {
            layer.instances.clear();
            layer.garbage = 0;
            layer.dirty.clear();
            layer.full = true;
        }
    }

    fn free(&mut self, a: Alloc) {
        let layer = &mut self.layers[a.layer as usize];
        let (s, e) = (a.start as usize, (a.start + a.len) as usize);
        for slot in &mut layer.instances[s..e] {
            *slot = Instance::EMPTY;
        }
        layer.garbage += a.len as usize;
        layer.mark_dirty(s, e);
    }

    /// Bounding box over all live instances.
    pub fn bounds(&self) -> Option<[f64; 4]> {
        let mut b: Option<[f32; 4]> = None;
        for layer in &self.layers {
            for inst in &layer.instances {
                if let Some(ib) = inst.bounds() {
                    b = Some(match b {
                        None => ib,
                        Some(b) => [b[0].min(ib[0]), b[1].min(ib[1]), b[2].max(ib[2]), b[3].max(ib[3])],
                    });
                }
            }
        }
        b.map(|b| b.map(|v| v as f64))
    }

    fn compact_layer(&mut self, layer_id: LayerId) {
        let layer = &mut self.layers[layer_id as usize];
        let mut out = Vec::with_capacity(layer.instances.len() - layer.garbage);
        for allocs in self.items.values_mut() {
            for a in allocs.iter_mut().filter(|a| a.layer == layer_id) {
                let (s, e) = (a.start as usize, (a.start + a.len) as usize);
                a.start = out.len() as u32;
                out.extend_from_slice(&layer.instances[s..e]);
            }
        }
        layer.instances = out;
        layer.garbage = 0;
        layer.full = true;
    }

    /// Collect everything that changed since the last call.
    pub fn take_upload(&mut self) -> SceneUpload {
        for id in 0..self.layers.len() {
            let l = &self.layers[id];
            if l.garbage > 4096 && l.garbage * 2 > l.instances.len() {
                self.compact_layer(id as LayerId);
            }
        }
        let layers = self
            .layers
            .iter_mut()
            .map(|layer| {
                let len = layer.instances.len();
                let data = if layer.full || len > layer.gpu_capacity {
                    let capacity = (len + len / 2).max(1024);
                    layer.gpu_capacity = capacity;
                    layer.dirty.clear();
                    LayerData::Full { capacity, instances: layer.instances.clone() }
                } else if !layer.dirty.is_empty() {
                    let ranges = layer.take_ranges();
                    LayerData::Partial(ranges.into_iter().map(|(s, e)| (s, layer.instances[s..e].to_vec())).collect())
                } else {
                    LayerData::Unchanged
                };
                layer.full = false;
                LayerUpload { len, data }
            })
            .collect();
        SceneUpload { layers }
    }

    /// A full upload of the current contents that leaves the incremental upload state
    /// alone (for rendering the same scene into a second, independent GPU copy).
    pub fn snapshot(&self) -> SceneUpload {
        let layers = self
            .layers
            .iter()
            .map(|layer| {
                let len = layer.instances.len();
                let capacity = len.max(1);
                LayerUpload { len, data: LayerData::Full { capacity, instances: layer.instances.clone() } }
            })
            .collect();
        SceneUpload { layers }
    }

    /// Forget GPU state so the next upload is a full one (e.g. after recreating the GPU scene).
    pub fn invalidate_gpu(&mut self) {
        for layer in &mut self.layers {
            layer.full = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primitive::{Color, kind};

    fn line(x: f32) -> Instance {
        Instance::line([x, 0.0], [x, 1.0], 0.1, Color::rgb(255, 0, 0))
    }

    fn live(scene: &Scene, layer: LayerId) -> Vec<f32> {
        scene.layers[layer as usize].instances.iter().filter(|i| i.kind != kind::EMPTY).map(|i| i.p0[0]).collect()
    }

    #[test]
    fn replace_reuses_slots_when_it_fits() {
        let mut s = Scene::new();
        let l = s.add_layer("a");
        s.set_item(1, [(l, line(1.0)), (l, line(2.0))]);
        s.set_item(2, [(l, line(3.0))]);
        let _ = s.take_upload();
        s.set_item(1, [(l, line(5.0))]);
        assert_eq!(s.slot_count(), 3);
        assert_eq!(live(&s, l), vec![5.0, 3.0]);
        let up = s.take_upload();
        match &up.layers[0].data {
            LayerData::Partial(ranges) => {
                assert_eq!(ranges.len(), 1);
                assert_eq!(ranges[0].0, 0);
                assert_eq!(ranges[0].1.len(), 2);
            }
            other => panic!("expected partial upload, got {other:?}"),
        }
    }

    #[test]
    fn distant_changes_upload_separately() {
        let mut s = Scene::new();
        let l = s.add_layer("a");
        for i in 0..5000u64 {
            s.set_item(i, [(l, line(i as f32))]);
        }
        let _ = s.take_upload();
        s.set_item(10, [(l, line(-1.0))]);
        s.set_item(9000, [(l, line(-2.0))]); // appended at the end
        let up = s.take_upload();
        match &up.layers[0].data {
            LayerData::Partial(ranges) => {
                assert_eq!(ranges.len(), 2);
                assert_eq!(up.bytes(), 2 * size_of::<Instance>());
            }
            other => panic!("expected partial upload, got {other:?}"),
        }
    }

    #[test]
    fn growing_item_appends_and_frees_old() {
        let mut s = Scene::new();
        let l = s.add_layer("a");
        s.set_item(1, [(l, line(1.0))]);
        s.set_item(1, [(l, line(1.0)), (l, line(2.0))]);
        assert_eq!(live(&s, l), vec![1.0, 2.0]);
        assert_eq!(s.instance_count(), 2);
        assert_eq!(s.slot_count(), 3);
    }

    #[test]
    fn compaction_preserves_items() {
        let mut s = Scene::new();
        let l = s.add_layer("a");
        for i in 0..10_000u64 {
            s.set_item(i, [(l, line(i as f32))]);
        }
        for i in 0..9_000u64 {
            s.remove_item(i);
        }
        let up = s.take_upload();
        assert!(matches!(up.layers[0].data, LayerData::Full { .. }));
        assert_eq!(s.slot_count(), 1000);
        // Items still map to the right slots after compaction.
        s.set_item(9_500, [(l, line(-1.0))]);
        assert!(live(&s, l).contains(&-1.0));
        assert!(!live(&s, l).contains(&9500.0));
    }
}
