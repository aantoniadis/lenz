// Ported from IntelliJ IDEA Community Edition (commit bc34875):
//   com.intellij.util.diff.Diff (buildChanges), Reindexer, MyersLCS.
// Copyright 2000-2025 JetBrains s.r.o. and contributors. Licensed under the Apache License, Version 2.0.
// Modified: translated from Kotlin to Rust; returns change ranges instead of a linked list; when the difference
// exceeds MyersLCS's threshold, gix-imara-diff's Myers is used instead of IntelliJ's PatienceIntLCS.
//
// Word and char refinement use this exact LCS because equal-length LCSs are common at that level and
// IntelliJ's tie-breaking decides which one is shown; the line level keeps gix-imara-diff.

use crate::by_line::R;

/// Changes between two token sequences, with IntelliJ's tie-breaking (Diff.buildChanges).
pub fn diff(a: &[u32], b: &[u32]) -> Vec<R> {
    let start = a.iter().zip(b).take_while(|(x, y)| x == y).count();
    let end = a[start..].iter().rev().zip(b[start..].iter().rev()).take_while(|(x, y)| x == y).count();
    let (a, b) = (&a[start..a.len() - end], &b[start..b.len() - end]);
    let whole = || vec![R { s1: start, e1: start + a.len(), s2: start, e2: start + b.len() }];
    if a.is_empty() && b.is_empty() {
        return Vec::new();
    }
    if a.is_empty() || b.is_empty() {
        return whole();
    }

    // Reindexer.discardUnique: elements with no counterpart can't be matched
    let max = a.iter().chain(b).copied().max().unwrap_or(0) as usize;
    let present = |s: &[u32]| {
        let mut p = vec![false; max + 1];
        s.iter().for_each(|&t| p[t as usize] = true);
        p
    };
    let in_b = present(b);
    let idx1: Vec<usize> = (0..a.len()).filter(|&i| in_b[a[i] as usize]).collect();
    let d1: Vec<u32> = idx1.iter().map(|&i| a[i]).collect();
    let in_d1 = present(&d1);
    let idx2: Vec<usize> = (0..b.len()).filter(|&i| in_d1[b[i] as usize]).collect();
    let d2: Vec<u32> = idx2.iter().map(|&i| b[i]).collect();
    if d1.is_empty() && d2.is_empty() {
        return whole();
    }

    let threshold = (20000 + 10 * ((d1.len() + d2.len()) as f64).sqrt() as usize).max(20000);
    let mut lcs = Myers::new(&d1, &d2);
    if !d1.is_empty() && !d2.is_empty() && lcs.execute(0, d1.len(), 0, d2.len(), threshold.min(d1.len() + d2.len())).is_err() {
        let changes = crate::by_line::diff(a, b, max as u32 + 1);
        return changes.into_iter().map(|c| R { s1: c.s1 + start, e1: c.e1 + start, s2: c.s2 + start, e2: c.e2 + start }).collect();
    }
    let (ch1, ch2) = reindex(&lcs.changes1, &lcs.changes2, &idx1, &idx2, a.len(), b.len());

    let mut out = Vec::new();
    let (n1, n2) = (a.len(), b.len());
    let (mut x, mut y) = (0, 0);
    let mut change = |x: usize, y: usize, dx: usize, dy: usize| {
        out.push(R { s1: start + x, e1: start + x + dx, s2: start + y, e2: start + y + dy });
    };
    while x < n1 && y < n2 {
        while x < n1 && y < n2 && !ch1[x] && !ch2[y] {
            x += 1;
            y += 1;
        }
        let (x0, y0) = (x, y);
        while x < n1 && ch1[x] {
            x += 1;
        }
        while y < n2 && ch2[y] {
            y += 1;
        }
        if x != x0 || y != y0 {
            change(x0, y0, x - x0, y - y0);
        }
    }
    if x != n1 || y != n2 {
        change(x, y, n1 - x, n2 - y);
    }
    out
}

/// Reindexer.reindex: changed flags on the discarded sequences back onto the originals.
fn reindex(dc1: &[bool], dc2: &[bool], idx1: &[usize], idx2: &[usize], n1: usize, n2: usize) -> (Vec<bool>, Vec<bool>) {
    let (m1, m2) = (idx1.len(), idx2.len());
    if m1 == n1 && m2 == n2 {
        return (dc1.to_vec(), dc2.to_vec());
    }
    let (mut ch1, mut ch2) = (vec![false; n1], vec![false; n2]);
    let increment = |idx: &[usize], i: usize, set: &mut [bool], len: usize| {
        let to = if i + 1 < idx.len() { idx[i + 1] } else { len };
        set[idx[i] + 1..to].iter_mut().for_each(|c| *c = true);
        i + 1
    };
    let (mut x, mut y) = (0, 0);
    while x < m1 || y < m2 {
        if x < m1 && y < m2 && !dc1[x] && !dc2[y] {
            x = increment(idx1, x, &mut ch1, n1);
            y = increment(idx2, y, &mut ch2, n2);
        } else if x < m1 && dc1[x] {
            ch1[idx1[x]] = true;
            x = increment(idx1, x, &mut ch1, n1);
        } else if y < m2 && dc2[y] {
            ch2[idx2[y]] = true;
            y = increment(idx2, y, &mut ch2, n2);
        } else {
            unreachable!("matched elements left on one side only");
        }
    }
    let head1 = if m1 == 0 { n1 } else { idx1[0] };
    let head2 = if m2 == 0 { n2 } else { idx2[0] };
    ch1[..head1].iter_mut().for_each(|c| *c = true);
    ch2[..head2].iter_mut().for_each(|c| *c = true);
    (ch1, ch2)
}

struct TooBig;

/// E.W. Myers, "An O(ND) Difference Algorithm and Its Variations" (1986), linear space (MyersLCS).
struct Myers<'a> {
    a: &'a [u32],
    b: &'a [u32],
    fwd: Vec<isize>,
    bwd: Vec<isize>,
    changes1: Vec<bool>,
    changes2: Vec<bool>,
}

impl<'a> Myers<'a> {
    fn new(a: &'a [u32], b: &'a [u32]) -> Self {
        let n = a.len() + b.len() + 1;
        Myers { a, b, fwd: vec![0; n], bwd: vec![0; n], changes1: vec![true; a.len()], changes2: vec![true; b.len()] }
    }

    /// LCS of `a[old_start..old_end)` and `b[new_start..new_end)`.
    fn execute(&mut self, old_start: usize, old_end: usize, new_start: usize, new_end: usize, estimate: usize) -> Result<(), TooBig> {
        if old_start >= old_end || new_start >= new_end {
            return Ok(());
        }
        let (os, oe, ns, ne) = (old_start as isize, old_end as isize, new_start as isize, new_end as isize);
        let (old_len, new_len) = (oe - os, ne - ns);
        self.fwd[(new_len + 1) as usize] = 0;
        self.bwd[(new_len + 1) as usize] = 0;
        let half_d = (estimate as isize + 1) / 2;
        let (mut xx, mut kk, mut td) = (-1isize, -1isize, -1isize);

        'search: for d in 0..=half_d {
            let l = new_len + (-d).max(-new_len + ((d ^ new_len) & 1));
            let r = new_len + d.min(old_len - ((d ^ old_len) & 1));
            let mut k = l;
            while k <= r {
                let (f, ku) = (&self.fwd, k as usize);
                let mut x = if k == l || k != r && f[ku - 1] < f[ku + 1] { f[ku + 1] } else { f[ku - 1] + 1 };
                let y = x - k + new_len;
                x += self.common_forward(os + x, ns + y, (oe - os - x).min(ne - ns - y));
                self.fwd[ku] = x;
                k += 2;
            }
            if (old_len - new_len) % 2 != 0 {
                let mut k = l;
                while k <= r {
                    if old_len - (d - 1) <= k
                        && k <= old_len + (d - 1)
                        && self.fwd[k as usize] + self.bwd[(new_len + old_len - k) as usize] >= old_len
                    {
                        (xx, kk, td) = (self.fwd[k as usize], k, 2 * d - 1);
                        break 'search;
                    }
                    k += 2;
                }
            }
            let mut k = l;
            while k <= r {
                let (bw, ku) = (&self.bwd, k as usize);
                let mut x = if k == l || k != r && bw[ku - 1] < bw[ku + 1] { bw[ku + 1] } else { bw[ku - 1] + 1 };
                let y = x - k + new_len;
                x += self.common_backward(oe - 1 - x, ne - 1 - y, (oe - os - x).min(ne - ns - y));
                self.bwd[ku] = x;
                k += 2;
            }
            if (old_len - new_len) % 2 == 0 {
                let mut k = l;
                while k <= r {
                    if old_len - d <= k && k <= old_len + d && self.fwd[(old_len + new_len - k) as usize] + self.bwd[k as usize] >= old_len {
                        (xx, kk, td) = (old_len - self.bwd[k as usize], old_len + new_len - k, 2 * d);
                        break 'search;
                    }
                    k += 2;
                }
            }
        }

        if td > 1 {
            let yy = xx - kk + new_len;
            let old_diff = (td + 1) / 2;
            if 0 < xx && 0 < yy {
                self.execute(old_start, (os + xx) as usize, new_start, (ns + yy) as usize, old_diff as usize)?;
            }
            if os + xx < oe && ns + yy < ne {
                self.execute((os + xx) as usize, old_end, (ns + yy) as usize, new_end, (td - old_diff) as usize)?;
            }
        } else if td >= 0 {
            let (mut x, mut y) = (os, ns);
            while x < oe && y < ne {
                let common = self.common_forward(x, y, (oe - x).min(ne - y));
                if common > 0 {
                    self.changes1[x as usize..(x + common) as usize].iter_mut().for_each(|c| *c = false);
                    self.changes2[y as usize..(y + common) as usize].iter_mut().for_each(|c| *c = false);
                    x += common;
                    y += common;
                } else if oe - os > ne - ns {
                    x += 1;
                } else {
                    y += 1;
                }
            }
        } else {
            return Err(TooBig);
        }
        Ok(())
    }

    fn common_forward(&self, old: isize, new: isize, max: isize) -> isize {
        let max = max.min((self.a.len() as isize - old).min(self.b.len() as isize - new));
        let mut n = 0;
        while n < max && self.a[(old + n) as usize] == self.b[(new + n) as usize] {
            n += 1;
        }
        n
    }

    fn common_backward(&self, old: isize, new: isize, max: isize) -> isize {
        let max = max.min(old.min(new) + 1);
        let mut n = 0;
        while n < max && self.a[(old - n) as usize] == self.b[(new - n) as usize] {
            n += 1;
        }
        n
    }
}
