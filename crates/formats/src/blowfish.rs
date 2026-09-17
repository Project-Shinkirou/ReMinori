use crate::blowfish_consts::{P_INIT, S0_INIT, S1_INIT, S2_INIT, S3_INIT};
pub struct Blowfish {
    p: [u32; 18],
    s: [[u32; 256]; 4],
}
#[inline]
fn f(s: &[[u32; 256]; 4], x: u32) -> u32 {
    s[3][(x & 0xFF) as usize]
        .wrapping_add(
            s[2][((x >> 8) & 0xFF) as usize]
                ^ s[0][(x >> 24) as usize]
                    .wrapping_add(s[1][((x >> 16) & 0xFF) as usize]),
        )
}
impl Blowfish {
    pub fn new(key: &[u8]) -> Self {
        let mut bf = Blowfish {
            p: P_INIT,
            s: [S0_INIT, S1_INIT, S2_INIT, S3_INIT],
        };
        let mut ki = 0usize;
        for i in 0..18 {
            let mut v: u32 = 0;
            for _ in 0..4 {
                v = (v << 8) | key[ki % key.len()] as u32;
                ki += 1;
            }
            bf.p[i] ^= v;
        }
        let (mut l, mut r) = (0u32, 0u32);
        for i in 0..9 {
            let (nl, nr) = bf.enc(l, r);
            l = nl;
            r = nr;
            bf.p[2 * i] = l;
            bf.p[2 * i + 1] = r;
        }
        for bi in 0..4 {
            for i in 0..128 {
                let (nl, nr) = bf.enc(l, r);
                l = nl;
                r = nr;
                bf.s[bi][2 * i] = l;
                bf.s[bi][2 * i + 1] = r;
            }
        }
        bf
    }
    fn enc(&self, l: u32, r: u32) -> (u32, u32) {
        let mut a = l ^ self.p[0];
        let mut b = r ^ f(&self.s, a) ^ self.p[1];
        for k in 2..16 {
            let cur = self.p[k] ^ f(&self.s, b) ^ a;
            a = b;
            b = cur;
        }
        let (v19, v18) = (b, a);
        (v19 ^ self.p[17], f(&self.s, v19) ^ v18 ^ self.p[16])
    }
    pub fn encrypt(&self, buf: &mut [u8]) {
        let n = buf.len() - buf.len() % 8;
        let mut i = 0;
        while i < n {
            let (l, r) = (
                u32::from_le_bytes(buf[i..i + 4].try_into().unwrap()),
                u32::from_le_bytes(buf[i + 4..i + 8].try_into().unwrap()),
            );
            let (el, er) = self.enc(l, r);
            buf[i..i + 4].copy_from_slice(&el.to_le_bytes());
            buf[i + 4..i + 8].copy_from_slice(&er.to_le_bytes());
            i += 8;
        }
    }
    pub fn dec(&self, l: u32, r: u32) -> (u32, u32) {
        let mut v = [0u32; 16];
        v[0] = self.p[17] ^ l;
        v[1] = self.p[16] ^ (r ^ f(&self.s, v[0]));
        let mut idx = 2;
        for k in (2..=15).rev() {
            v[idx] = self.p[k] ^ f(&self.s, v[idx - 1]) ^ v[idx - 2];
            idx += 1;
        }
        let v24 = v[idx - 1];
        let v18 = f(&self.s, v24) ^ v[idx - 2];
        (v24 ^ self.p[0], v18 ^ self.p[1])
    }
    pub fn decrypt(&self, buf: &mut [u8]) {
        let n = buf.len() - buf.len() % 8;
        let mut i = 0;
        while i < n {
            let (l, r) = (
                u32::from_le_bytes(buf[i..i + 4].try_into().unwrap()),
                u32::from_le_bytes(buf[i + 4..i + 8].try_into().unwrap()),
            );
            let (dl, dr) = self.dec(l, r);
            buf[i..i + 4].copy_from_slice(&dl.to_le_bytes());
            buf[i + 4..i + 8].copy_from_slice(&dr.to_le_bytes());
            i += 8;
        }
    }
}
pub fn negate_key(key: &[u8; 32]) -> [u8; 32] {
    let mut out = [0u8; 32];
    for (o, &b) in out.iter_mut().zip(key.iter()) {
        *o = b.wrapping_neg();
    }
    out
}
