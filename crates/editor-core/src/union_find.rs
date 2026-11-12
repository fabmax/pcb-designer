//! Disjoint sets over `0..n` with path halving. The root of a set is its smallest
//! member, so it doesn't depend on the order of the unions.

pub(crate) struct UnionFind(Vec<u32>);

impl UnionFind {
    pub(crate) fn new(n: usize) -> Self {
        UnionFind((0..n as u32).collect())
    }

    pub(crate) fn find(&mut self, mut x: usize) -> usize {
        while self.0[x] as usize != x {
            let p = self.0[x] as usize;
            self.0[x] = self.0[p];
            x = p;
        }
        x
    }

    pub(crate) fn union(&mut self, a: usize, b: usize) {
        let (a, b) = (self.find(a), self.find(b));
        if a != b {
            self.0[a.max(b)] = a.min(b) as u32;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::UnionFind;

    #[test]
    fn root_is_the_smallest_member() {
        let mut uf = UnionFind::new(6);
        uf.union(4, 5);
        uf.union(5, 2);
        uf.union(0, 1);
        assert_eq!([0, 1, 2, 3, 4, 5].map(|i| uf.find(i)), [0, 0, 2, 3, 2, 2]);
        uf.union(1, 4);
        assert!((0..6).filter(|&i| i != 3).all(|i| uf.find(i) == 0));
    }
}
