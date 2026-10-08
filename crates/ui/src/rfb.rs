//! Bounded RFB 3.8 reader. Inputs use the backend, never the display connection.
use anyhow::{Result, bail, ensure};
use std::io::{Read, Write};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Damage {
    pub origin: [usize; 2],
    pub size: [usize; 2],
}
impl Damage {
    pub fn union(self, other: Self) -> Self {
        let origin = [
            self.origin[0].min(other.origin[0]),
            self.origin[1].min(other.origin[1]),
        ];
        let end = [
            (self.origin[0] + self.size[0]).max(other.origin[0] + other.size[0]),
            (self.origin[1] + self.size[1]).max(other.origin[1] + other.size[1]),
        ];
        Self {
            origin,
            size: [end[0] - origin[0], end[1] - origin[1]],
        }
    }
}

pub struct Framebuffer {
    pub width: usize,
    pub height: usize,
    pixels: Vec<u8>,
    damage: Option<Damage>,
}
impl Framebuffer {
    pub fn new(width: usize, height: usize) -> Result<Self> {
        ensure!(
            width > 0 && height > 0 && width <= 4096 && height <= 4096,
            "Invalid display dimensions"
        );
        let mut pixels = vec![0; width * height * 4];
        for pixel in pixels.chunks_exact_mut(4) {
            pixel[3] = 255;
        }
        Ok(Self {
            width,
            height,
            pixels,
            damage: Some(Damage {
                origin: [0, 0],
                size: [width, height],
            }),
        })
    }
    pub fn raw(&mut self, x: usize, y: usize, w: usize, h: usize, raw: &[u8]) -> Result<()> {
        ensure!(
            w > 0
                && h > 0
                && w <= self.width
                && h <= self.height
                && x <= self.width - w
                && y <= self.height - h,
            "Invalid display rectangle"
        );
        ensure!(raw.len() == w * h * 4, "Truncated display rectangle");
        let damage = Damage {
            origin: [x, y],
            size: [w, h],
        };
        self.damage = Some(self.damage.map_or(damage, |old| old.union(damage)));
        for row in 0..h {
            let dest = ((y + row) * self.width + x) * 4;
            for (out, input) in self.pixels[dest..dest + w * 4]
                .chunks_exact_mut(4)
                .zip(raw[row * w * 4..(row + 1) * w * 4].chunks_exact(4))
            {
                out.copy_from_slice(&[input[2], input[1], input[0], 255]);
            }
        }
        Ok(())
    }
    pub fn rgba(&self) -> &[u8] {
        &self.pixels
    }
    pub fn take_damage(&mut self) -> Option<Damage> {
        self.damage.take()
    }
    pub fn copy_damage(&self, damage: Damage) -> Vec<u8> {
        let [x, y] = damage.origin;
        let [width, height] = damage.size;
        if width == self.width {
            let start = y * self.width * 4;
            return self.pixels[start..start + width * height * 4].to_vec();
        }
        let mut pixels = Vec::with_capacity(width * height * 4);
        for row in y..y + height {
            let start = (row * self.width + x) * 4;
            pixels.extend_from_slice(&self.pixels[start..start + width * 4]);
        }
        pixels
    }
}
fn bytes<const N: usize>(stream: &mut impl Read) -> Result<[u8; N]> {
    let mut out = [0; N];
    stream.read_exact(&mut out)?;
    Ok(out)
}
fn skip(stream: &mut impl Read, length: usize) -> Result<()> {
    let mut limited = stream.take(length as u64);
    let copied = std::io::copy(&mut limited, &mut std::io::sink())?;
    ensure!(copied == length as u64, "Truncated display message");
    Ok(())
}
pub fn handshake(stream: &mut (impl Read + Write)) -> Result<Framebuffer> {
    let version = bytes::<12>(stream)?;
    ensure!(&version == b"RFB 003.008\n", "Unsupported display protocol");
    stream.write_all(&version)?;
    let count = bytes::<1>(stream)?[0] as usize;
    ensure!(count > 0, "Display rejected connection");
    let mut security = vec![0; count];
    stream.read_exact(&mut security)?;
    ensure!(
        security.contains(&1),
        "Display authentication is unsupported"
    );
    stream.write_all(&[1])?;
    ensure!(
        u32::from_be_bytes(bytes(stream)?) == 0,
        "Display authentication failed"
    );
    stream.write_all(&[1])?;
    let init = bytes::<24>(stream)?;
    let fb = Framebuffer::new(
        u16::from_be_bytes([init[0], init[1]]) as usize,
        u16::from_be_bytes([init[2], init[3]]) as usize,
    )?;
    let name = u32::from_be_bytes(init[20..24].try_into().unwrap()) as usize;
    ensure!(name <= 65536, "Invalid display name");
    skip(stream, name)?;
    stream.write_all(&[
        0, 0, 0, 0, 32, 24, 0, 1, 0, 255, 0, 255, 0, 255, 16, 8, 0, 0, 0, 0,
    ])?;
    stream.write_all(&[2, 0, 0, 2, 0, 0, 0, 0, 255, 255, 255, 33])?;
    request(stream, &fb, false)?;
    Ok(fb)
}
fn request(stream: &mut impl Write, fb: &Framebuffer, incremental: bool) -> Result<()> {
    let mut command = vec![3, u8::from(incremental), 0, 0, 0, 0];
    command.extend_from_slice(&(fb.width as u16).to_be_bytes());
    command.extend_from_slice(&(fb.height as u16).to_be_bytes());
    stream.write_all(&command)?;
    stream.flush()?;
    Ok(())
}
pub fn update(stream: &mut (impl Read + Write), fb: &mut Framebuffer) -> Result<bool> {
    match bytes::<1>(stream)?[0] {
        0 => {
            let head = bytes::<3>(stream)?;
            let count = u16::from_be_bytes([head[1], head[2]]) as usize;
            ensure!(count <= 4096, "Too many display rectangles");
            let mut budget = 0;
            for _ in 0..count {
                let r = bytes::<12>(stream)?;
                let word = |n| u16::from_be_bytes([r[n], r[n + 1]]) as usize;
                let (x, y, w, h) = (word(0), word(2), word(4), word(6));
                let encoding = i32::from_be_bytes(r[8..12].try_into().unwrap());
                if encoding == -223 {
                    *fb = Framebuffer::new(w, h)?;
                    continue;
                }
                ensure!(encoding == 0, "Unsupported display encoding");
                ensure!(
                    w > 0
                        && h > 0
                        && w <= fb.width
                        && h <= fb.height
                        && x <= fb.width - w
                        && y <= fb.height - h,
                    "Invalid display rectangle"
                );
                budget += w * h * 4;
                ensure!(budget <= 128 * 1024 * 1024, "Display update too large");
                let mut raw = vec![0; w * h * 4];
                stream.read_exact(&mut raw)?;
                fb.raw(x, y, w, h, &raw)?;
            }
            request(stream, fb, true)?;
            Ok(count > 0)
        }
        2 => Ok(false),
        3 => {
            let head = bytes::<7>(stream)?;
            let length = u32::from_be_bytes(head[3..7].try_into().unwrap()) as usize;
            ensure!(length <= 1_048_576, "Invalid clipboard length");
            skip(stream, length)?;
            Ok(false)
        }
        _ => bail!("Unsupported display message"),
    }
}
