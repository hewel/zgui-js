import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { inflateSync } from "node:zlib";

export async function pngRgba(file) {
  const bytes = await readFile(file);
  assert.deepEqual(bytes.subarray(0, 8), Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]));
  let width = 0,
    height = 0;
  const chunks = [];
  for (let offset = 8; offset + 12 <= bytes.length;) {
    const length = bytes.readUInt32BE(offset);
    assert.ok(offset + 12 + length <= bytes.length);
    const type = bytes.toString("ascii", offset + 4, offset + 8);
    const chunk = bytes.subarray(offset + 8, offset + 8 + length);
    if (type === "IHDR") {
      assert.equal(length, 13);
      width = chunk.readUInt32BE(0);
      height = chunk.readUInt32BE(4);
      assert.ok(width > 0 && height > 0 && width <= 16384 && height <= 16384);
      assert.deepEqual([...chunk.subarray(8)], [8, 6, 0, 0, 0]);
    } else if (type === "IDAT") chunks.push(chunk);
    offset += length + 12;
    if (type === "IEND") break;
  }
  assert.ok(width && height && chunks.length);
  const stride = width * 4;
  const raw = inflateSync(Buffer.concat(chunks), { maxOutputLength: (stride + 1) * height });
  assert.equal(raw.length, (stride + 1) * height);
  const data = Buffer.alloc(stride * height);
  const paeth = (a, b, c) => {
    const p = a + b - c,
      pa = Math.abs(p - a),
      pb = Math.abs(p - b),
      pc = Math.abs(p - c);
    return pa <= pb && pa <= pc ? a : pb <= pc ? b : c;
  };
  for (let y = 0; y < height; y++) {
    const filter = raw[y * (stride + 1)];
    assert.ok(filter <= 4);
    for (let x = 0; x < stride; x++) {
      const index = y * stride + x;
      const a = x >= 4 ? data[index - 4] : 0,
        b = y ? data[index - stride] : 0,
        c = y && x >= 4 ? data[index - stride - 4] : 0;
      const prediction =
        filter === 0
          ? 0
          : filter === 1
            ? a
            : filter === 2
              ? b
              : filter === 3
                ? Math.floor((a + b) / 2)
                : paeth(a, b, c);
      data[index] = (raw[y * (stride + 1) + 1 + x] + prediction) & 255;
    }
  }
  return { width, height, data };
}
