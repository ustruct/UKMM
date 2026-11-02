const BRTI_HEADER: usize = 0xA0;
const TEX_OBJECT: usize = 0x100;
const MEMORY_POOL_OFFSET: usize = 0x58;
const MEMORY_POOL_SIZE: usize = 0x140;
const INFO_PTRS_OFFSET: usize = MEMORY_POOL_OFFSET + MEMORY_POOL_SIZE;

#[derive(Debug, Clone, PartialEq)]
pub struct Texture {
    pub name: String,

    info: Vec<u8>,
    tex_obj: Vec<u8>,
    tex_view: Vec<u8>,

    mips: Vec<u64>,
    alignment: usize,
    data: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Bntx {
    header: [u8; 0x10],
    pub file_name: String,
    memory_pool: Vec<u8>,
    pub textures: Vec<Texture>,
}

#[inline]
fn align(x: usize, a: usize) -> usize {
    if a <= 1 { x } else { x.div_ceil(a) * a }
}

struct Reader<'a>(&'a [u8]);

impl Reader<'_> {
    fn slice(&self, off: usize, len: usize) -> Option<&[u8]> {
        self.0.get(off..off.checked_add(len)?)
    }

    fn u16(&self, off: usize) -> Option<u16> {
        Some(u16::from_le_bytes(self.slice(off, 2)?.try_into().ok()?))
    }

    fn u32(&self, off: usize) -> Option<u32> {
        Some(u32::from_le_bytes(self.slice(off, 4)?.try_into().ok()?))
    }

    fn u64(&self, off: usize) -> Option<usize> {
        usize::try_from(u64::from_le_bytes(self.slice(off, 8)?.try_into().ok()?)).ok()
    }

    fn string(&self, off: usize) -> Option<String> {
        let len = self.u16(off)? as usize;
        Some(String::from_utf8_lossy(self.slice(off + 2, len)?).into_owned())
    }
}

impl Bntx {
    pub fn parse(data: &[u8]) -> Option<Self> {
        let r = Reader(data);
        if r.slice(0, 4)? != b"BNTX" || r.slice(0x20, 4)? != b"NX  " {
            return None;
        }

        if r.u16(0x0C)? != 0xFEFF || data[0x0F] != 0x40 {
            return None;
        }
        let name_off = r.u32(0x10)? as usize;
        let file_name = r.string(name_off.checked_sub(2)?)?;
        let count = r.u32(0x24)? as usize;
        let info_ptrs = r.u64(0x28)?;
        let memory_pool_off = r.u64(0x40)?;
        let memory_pool = r.slice(memory_pool_off, MEMORY_POOL_SIZE)?.to_vec();
        let mut textures = Vec::with_capacity(count);
        for i in 0..count {
            let q = r.u64(info_ptrs + 8 * i)?;
            if r.slice(q, 4)? != b"BRTI" {
                return None;
            }
            let mip_count = r.u16(q + 0x16)? as usize;
            let size = r.u32(q + 0x50)? as usize;
            let alignment = r.u32(q + 0x54)? as usize;
            let name = r.string(r.u64(q + 0x60)?)?;
            let mips_off = r.u64(q + 0x70)?;

            if r.u64(q + 0x78)? != 0 || r.u64(q + 0x98)? != 0 {
                return None;
            }
            let tex_obj = r.slice(r.u64(q + 0x80)?, TEX_OBJECT)?.to_vec();
            let tex_view = r.slice(r.u64(q + 0x88)?, TEX_OBJECT)?.to_vec();
            let mut mips = Vec::with_capacity(mip_count);
            for m in 0..mip_count {
                mips.push(r.u64(mips_off + 8 * m)?);
            }
            let start = *mips.first()?;
            let mips = mips
                .into_iter()
                .map(|m| m.checked_sub(start).map(|m| m as u64))
                .collect::<Option<Vec<_>>>()?;
            textures.push(Texture {
                name,
                info: r.slice(q + 0x10, 0x50)?.to_vec(),
                tex_obj,
                tex_view,
                mips,
                alignment,
                data: r.slice(start, size)?.to_vec(),
            });
        }
        Some(Self {
            header: data[..0x10].try_into().ok()?,
            file_name,
            memory_pool,
            textures,
        })
    }

    pub fn merge(&mut self, other: Bntx) {
        for tex in other.textures {
            match self.textures.iter_mut().find(|t| t.name == tex.name) {
                Some(t) => *t = tex,
                None => self.textures.push(tex),
            }
        }
        self.textures
            .sort_by(|a, b| a.name.as_bytes().cmp(b.name.as_bytes()));
    }

    pub fn to_binary(&self) -> Vec<u8> {
        let texs = &self.textures;
        let n = texs.len();
        let file_align = 1usize << self.header[0x0E].min(16);

        let str_off = align(INFO_PTRS_OFFSET + 8 * n, 8);
        let mut strs: Vec<u8> = Vec::new();
        strs.extend_from_slice(b"_STR");
        strs.extend_from_slice(&[0u8; 12]);
        strs.extend_from_slice(&((n + 1) as u32).to_le_bytes());
        let add_str = |strs: &mut Vec<u8>, s: &str| -> usize {
            let at = str_off + strs.len();
            strs.extend_from_slice(&(s.len() as u16).to_le_bytes());
            strs.extend_from_slice(s.as_bytes());
            strs.push(0);
            if strs.len() % 2 == 1 {
                strs.push(0);
            }
            at
        };
        let empty_str = add_str(&mut strs, "");
        let file_name_str = add_str(&mut strs, &self.file_name);
        let tex_strs: Vec<usize> = texs.iter().map(|t| add_str(&mut strs, &t.name)).collect();
        strs.resize(align(strs.len(), 8), 0);
        let dict_off = str_off + strs.len();
        let nodes = build_dict(texs.iter().map(|t| t.name.as_bytes()));
        strs.extend_from_slice(b"_DIC");
        strs.extend_from_slice(&(n as u32).to_le_bytes());
        for (i, node) in nodes.iter().enumerate() {
            strs.extend_from_slice(&node.reference.to_le_bytes());
            strs.extend_from_slice(&(node.left as u16).to_le_bytes());
            strs.extend_from_slice(&(node.right as u16).to_le_bytes());
            let name = if i == 0 { empty_str } else { tex_strs[i - 1] };
            strs.extend_from_slice(&(name as u64).to_le_bytes());
        }
        let str_block = strs.len() as u32;
        strs[4..8].copy_from_slice(&str_block.to_le_bytes());
        strs[8..12].copy_from_slice(&str_block.to_le_bytes());

        let first_brti = str_off + strs.len();
        let mut brtis = Vec::with_capacity(n);
        let mut p = first_brti;
        for t in texs {
            brtis.push(p);
            p = align(p + BRTI_HEADER + 2 * TEX_OBJECT + 8 * t.mips.len(), 8);
        }
        let section1_end = p;
        let brtd = align(section1_end + 0x10, file_align) - 0x10;
        let mut data_offs = Vec::with_capacity(n);
        let mut dp = brtd + 0x10;
        for t in texs {
            dp = align(dp, t.alignment.max(1));
            data_offs.push(dp);
            dp += t.data.len();
        }
        let rlt = align(dp, file_align);

        let mut buf = vec![0u8; rlt];
        let put = |buf: &mut Vec<u8>, off: usize, bytes: &[u8]| {
            buf[off..off + bytes.len()].copy_from_slice(bytes);
        };
        put(&mut buf, 0, &self.header);
        put(&mut buf, MEMORY_POOL_OFFSET, &self.memory_pool);
        put(&mut buf, str_off, &strs);
        for (i, t) in texs.iter().enumerate() {
            let q = brtis[i];
            let block = (if i + 1 < n { brtis[i + 1] } else { brtd } - q) as u32;
            put(
                &mut buf,
                INFO_PTRS_OFFSET + 8 * i,
                &(q as u64).to_le_bytes(),
            );
            put(&mut buf, q, b"BRTI");
            put(&mut buf, q + 4, &block.to_le_bytes());
            put(&mut buf, q + 8, &block.to_le_bytes());
            put(&mut buf, q + 0x10, &t.info);
            let ptrs = [
                tex_strs[i] as u64,
                0x20,
                (q + BRTI_HEADER + 2 * TEX_OBJECT) as u64,
                0,
                (q + BRTI_HEADER) as u64,
                (q + BRTI_HEADER + TEX_OBJECT) as u64,
                0,
                0,
            ];
            for (j, v) in ptrs.iter().enumerate() {
                put(&mut buf, q + 0x60 + 8 * j, &v.to_le_bytes());
            }
            put(&mut buf, q + BRTI_HEADER, &t.tex_obj);
            put(&mut buf, q + BRTI_HEADER + TEX_OBJECT, &t.tex_view);
            for (j, m) in t.mips.iter().enumerate() {
                let at = q + BRTI_HEADER + 2 * TEX_OBJECT + 8 * j;
                put(&mut buf, at, &(data_offs[i] as u64 + m).to_le_bytes());
            }
            put(&mut buf, data_offs[i], &t.data);
        }
        put(&mut buf, brtd, b"BRTD");
        put(&mut buf, brtd + 8, &((rlt - brtd) as u32).to_le_bytes());

        fn run(entries: &mut Vec<(u32, u16, u8, u8)>, mut off: usize, mut count: usize) {
            while count > 0 {
                let c = count.min(255);
                entries.push((off as u32, 1, c as u8, 0));
                off += 8 * c;
                count -= c;
            }
        }
        let mut e1: Vec<(u32, u16, u8, u8)> = vec![(0x28, 2, 1, 1), (0x40, 1, 1, 0)];
        run(&mut e1, INFO_PTRS_OFFSET, n);
        e1.push(((dict_off + 0x10) as u32, (n + 1) as u16, 1, 1));
        for &q in &brtis {
            let q = q as u32;
            e1.extend([
                (q + 0x60, 1, 3, 0),
                (q + 0x78, 1, 1, 0),
                (q + 0x80, 1, 2, 0),
                (q + 0x98, 1, 1, 0),
            ]);
        }
        let mut e2: Vec<(u32, u16, u8, u8)> = vec![(0x30, 1, 1, 0)];
        for (q, t) in brtis.iter().zip(texs) {
            run(&mut e2, q + BRTI_HEADER + 2 * TEX_OBJECT, t.mips.len());
        }
        buf.extend_from_slice(b"_RLT");
        buf.extend_from_slice(&(rlt as u32).to_le_bytes());
        buf.extend_from_slice(&2u32.to_le_bytes());
        buf.extend_from_slice(&0u32.to_le_bytes());
        for (off, size, index, count) in [
            (0usize, section1_end, 0usize, e1.len()),
            (brtd, rlt - brtd, e1.len(), e2.len()),
        ] {
            buf.extend_from_slice(&0u64.to_le_bytes());
            buf.extend_from_slice(&(off as u32).to_le_bytes());
            buf.extend_from_slice(&(size as u32).to_le_bytes());
            buf.extend_from_slice(&(index as u32).to_le_bytes());
            buf.extend_from_slice(&(count as u32).to_le_bytes());
        }
        for (off, structs, offsets, padding) in e1.into_iter().chain(e2) {
            buf.extend_from_slice(&off.to_le_bytes());
            buf.extend_from_slice(&structs.to_le_bytes());
            buf.push(offsets);
            buf.push(padding);
        }

        let file_size = buf.len() as u32;
        put(&mut buf, 0x10, &((file_name_str + 2) as u32).to_le_bytes());
        put(&mut buf, 0x14, &0u16.to_le_bytes());
        put(&mut buf, 0x16, &(str_off as u16).to_le_bytes());
        put(&mut buf, 0x18, &(rlt as u32).to_le_bytes());
        put(&mut buf, 0x1C, &file_size.to_le_bytes());
        put(&mut buf, 0x20, b"NX  ");
        put(&mut buf, 0x24, &(n as u32).to_le_bytes());
        put(&mut buf, 0x28, &(INFO_PTRS_OFFSET as u64).to_le_bytes());
        put(&mut buf, 0x30, &(brtd as u64).to_le_bytes());
        put(&mut buf, 0x38, &(dict_off as u64).to_le_bytes());
        put(&mut buf, 0x40, &(MEMORY_POOL_OFFSET as u64).to_le_bytes());
        put(&mut buf, 0x48, &0u64.to_le_bytes());
        put(&mut buf, 0x50, &0u64.to_le_bytes());
        buf
    }
}

struct DictNode<'a> {
    reference: i32,
    left: usize,
    right: usize,
    key: &'a [u8],
}

fn key_bit(key: &[u8], n: i32) -> bool {
    if n < 0 {
        return false;
    }
    let byte = (n >> 3) as usize;
    if byte >= key.len() {
        return false;
    }
    (key[key.len() - 1 - byte] >> (n & 7)) & 1 == 1
}

fn build_dict<'a>(keys: impl Iterator<Item = &'a [u8]>) -> Vec<DictNode<'a>> {
    let mut nodes = vec![DictNode {
        reference: -1,
        left: 0,
        right: 0,
        key: b"",
    }];
    for key in keys {

        let mut p = 0;
        let mut x = nodes[0].left;
        while nodes[p].reference < nodes[x].reference {
            p = x;
            x = if key_bit(key, nodes[x].reference) {
                nodes[x].right
            } else {
                nodes[x].left
            };
        }
        let closest = nodes[x].key;
        let mut bit = 0i32;
        while key_bit(closest, bit) == key_bit(key, bit) {
            bit += 1;
            if bit > 8 * (closest.len().max(key.len()) as i32) + 8 {

                break;
            }
        }
        let mut p = 0;
        let mut t = nodes[0].left;
        while nodes[p].reference < nodes[t].reference && nodes[t].reference < bit {
            p = t;
            t = if key_bit(key, nodes[t].reference) {
                nodes[t].right
            } else {
                nodes[t].left
            };
        }
        let idx = nodes.len();
        let (left, right) = if key_bit(key, bit) {
            (t, idx)
        } else {
            (idx, t)
        };
        nodes.push(DictNode {
            reference: bit,
            left,
            right,
            key,
        });
        if p == 0 {
            nodes[0].left = idx;
        } else if key_bit(key, nodes[p].reference) {
            nodes[p].right = idx;
        } else {
            nodes[p].left = idx;
        }
    }
    nodes
}

pub fn merge_bntx(base: &[u8], diff: &[u8]) -> Option<Vec<u8>> {
    let mut base = Bntx::parse(base)?;
    let diff = Bntx::parse(diff)?;
    base.merge(diff);
    Some(base.to_binary())
}
