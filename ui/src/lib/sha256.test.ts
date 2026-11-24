import { describe, expect, it } from "vitest";

import { fillSeededBytes, Sha256Hasher, sha256Hex } from "./sha256";

const enc = new TextEncoder();

describe("Sha256Hasher", () => {
  it("matches standard NIST vectors", () => {
    expect(sha256Hex(new Uint8Array(0))).toBe(
      "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    );
    expect(sha256Hex(enc.encode("abc"))).toBe(
      "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
    );
    expect(sha256Hex(enc.encode("abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"))).toBe(
      "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1",
    );
  });

  it("matches single-shot hash across arbitrary chunk boundaries", () => {
    const buf = new Uint8Array(4099);
    fillSeededBytes(42, buf);
    const full = sha256Hex(buf);

    const h = new Sha256Hasher();
    const steps = [1, 7, 56, 63, 64, 65, 128, 1000];
    let at = 0;
    let si = 0;
    while (at < buf.length) {
      const len = Math.min(steps[si % steps.length] ?? 17, buf.length - at);
      h.update(buf.subarray(at, at + len));
      at += len;
      si++;
    }
    expect(h.digestHex()).toBe(full);
  });

  it("generates deterministic seeded bytes", () => {
    const a = new Uint8Array(259);
    const b = new Uint8Array(259);
    const c = new Uint8Array(259);
    fillSeededBytes(7, a);
    fillSeededBytes(7, b);
    fillSeededBytes(8, c);
    expect(a).toEqual(b);
    expect(a).not.toEqual(c);
  });
});
