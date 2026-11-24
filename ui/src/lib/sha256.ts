/**
 * Pure TypeScript streaming SHA-256 (RFC 6234) and deterministic byte-pattern helper.
 *
 * Works over plain HTTP LAN origins (such as http://<verdin-ip>:8080) where browser
 * `crypto.subtle` is unavailable outside secure contexts.
 */

const K = new Uint32Array([
  0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
  0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
  0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
  0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
  0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
  0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
  0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
  0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
]);

const H_INIT = new Uint32Array([
  0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
]);

function rotr(x: number, n: number): number {
  return (x >>> n) | (x << (32 - n));
}

export class Sha256Hasher {
  private readonly h = new Uint32Array(H_INIT);
  private readonly w = new Uint32Array(64);
  private readonly block = new Uint8Array(64);
  private blockLen = 0;
  private totalBytes = 0;
  private finalized = false;

  update(data: Uint8Array): this {
    if (this.finalized) {
      throw new Error("Sha256Hasher cannot be updated after digestHex()");
    }
    this.totalBytes += data.length;
    let offset = 0;
    if (this.blockLen > 0) {
      const take = Math.min(64 - this.blockLen, data.length);
      this.block.set(data.subarray(0, take), this.blockLen);
      this.blockLen += take;
      offset += take;
      if (this.blockLen === 64) {
        this.compress(this.block, 0);
        this.blockLen = 0;
      }
    }
    while (offset + 64 <= data.length) {
      this.compress(data, offset);
      offset += 64;
    }
    if (offset < data.length) {
      const rem = data.subarray(offset);
      this.block.set(rem, 0);
      this.blockLen = rem.length;
    }
    return this;
  }

  digestHex(): string {
    if (!this.finalized) {
      this.finalized = true;
      this.block[this.blockLen++] = 0x80;
      if (this.blockLen > 56) {
        this.block.fill(0, this.blockLen, 64);
        this.compress(this.block, 0);
        this.blockLen = 0;
      }
      this.block.fill(0, this.blockLen, 56);

      const hi = Math.floor(this.totalBytes / 0x20000000) >>> 0;
      const lo = (this.totalBytes << 3) >>> 0;
      const view = new DataView(this.block.buffer, this.block.byteOffset, 64);
      view.setUint32(56, hi, false);
      view.setUint32(60, lo, false);
      this.compress(this.block, 0);
    }

    let hex = "";
    for (let i = 0; i < 8; i++) {
      hex += (this.h[i] ?? 0).toString(16).padStart(8, "0");
    }
    return hex;
  }

  private compress(buf: Uint8Array, offset: number): void {
    const w = this.w;
    for (let i = 0; i < 16; i++) {
      const j = offset + i * 4;
      w[i] =
        (((buf[j] ?? 0) << 24) |
          ((buf[j + 1] ?? 0) << 16) |
          ((buf[j + 2] ?? 0) << 8) |
          (buf[j + 3] ?? 0)) >>>
        0;
    }
    for (let i = 16; i < 64; i++) {
      const w15 = w[i - 15] ?? 0;
      const w2 = w[i - 2] ?? 0;
      const s0 = rotr(w15, 7) ^ rotr(w15, 18) ^ (w15 >>> 3);
      const s1 = rotr(w2, 17) ^ rotr(w2, 19) ^ (w2 >>> 10);
      w[i] = ((w[i - 16] ?? 0) + s0 + (w[i - 7] ?? 0) + s1) >>> 0;
    }

    let a = this.h[0] ?? 0;
    let b = this.h[1] ?? 0;
    let c = this.h[2] ?? 0;
    let d = this.h[3] ?? 0;
    let e = this.h[4] ?? 0;
    let f = this.h[5] ?? 0;
    let g = this.h[6] ?? 0;
    let h = this.h[7] ?? 0;

    for (let i = 0; i < 64; i++) {
      const S1 = rotr(e, 6) ^ rotr(e, 11) ^ rotr(e, 25);
      const ch = (e & f) ^ (~e & g);
      const temp1 = (h + S1 + ch + (K[i] ?? 0) + (w[i] ?? 0)) >>> 0;
      const S0 = rotr(a, 2) ^ rotr(a, 13) ^ rotr(a, 22);
      const maj = (a & b) ^ (a & c) ^ (b & c);
      const temp2 = (S0 + maj) >>> 0;

      h = g;
      g = f;
      f = e;
      e = (d + temp1) >>> 0;
      d = c;
      c = b;
      b = a;
      a = (temp1 + temp2) >>> 0;
    }

    this.h[0] = ((this.h[0] ?? 0) + a) >>> 0;
    this.h[1] = ((this.h[1] ?? 0) + b) >>> 0;
    this.h[2] = ((this.h[2] ?? 0) + c) >>> 0;
    this.h[3] = ((this.h[3] ?? 0) + d) >>> 0;
    this.h[4] = ((this.h[4] ?? 0) + e) >>> 0;
    this.h[5] = ((this.h[5] ?? 0) + f) >>> 0;
    this.h[6] = ((this.h[6] ?? 0) + g) >>> 0;
    this.h[7] = ((this.h[7] ?? 0) + h) >>> 0;
  }
}

export function sha256Hex(data: Uint8Array): string {
  return new Sha256Hasher().update(data).digestHex();
}

/** Deterministic non-zero byte stream for throughput upload benchmarks. */
export function fillSeededBytes(seed: number, out: Uint8Array): void {
  let state = (seed ^ 0x9e3779b9) >>> 0 || 0x6d2b79f5;
  const words = out.length >>> 2;
  const view = new DataView(out.buffer, out.byteOffset, out.byteLength);
  for (let i = 0; i < words; i++) {
    state ^= state << 13;
    state ^= state >>> 17;
    state ^= state << 5;
    state = state >>> 0 || 1;
    view.setUint32(i * 4, Math.imul(state, 0x85ebca6b) >>> 0, true);
  }
  for (let i = words * 4; i < out.length; i++) {
    state ^= state << 13;
    state ^= state >>> 17;
    state ^= state << 5;
    out[i] = (state >>> 0) & 0xff;
  }
}
