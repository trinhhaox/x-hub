// H.264 helpers: scrcpy-style Annex-B packets -> WebCodecs AVCC chunks + avcC decoder config

const NAL_SPS = 7;
const NAL_PPS = 8;

const hex2 = (n: number) => n.toString(16).padStart(2, '0');

// Splits an Annex-B byte stream (00 00 01 / 00 00 00 01 start codes) into NAL units
function splitNals(data: Uint8Array): Uint8Array[] {
  const nals: Uint8Array[] = [];
  let start = -1;
  for (let i = 0; i + 2 < data.length; i++) {
    if (data[i] === 0 && data[i + 1] === 0 && data[i + 2] === 1) {
      if (start >= 0) nals.push(data.subarray(start, data[i - 1] === 0 ? i - 1 : i));
      start = i + 3;
      i += 2;
    }
  }
  if (start >= 0 && start < data.length) nals.push(data.subarray(start));
  return nals;
}

// Annex-B -> AVCC (4-byte big-endian length prefixes), the layout WebCodecs expects with a description
export function toAvcc(data: Uint8Array): Uint8Array {
  const nals = splitNals(data);
  const out = new Uint8Array(nals.reduce((n, nal) => n + 4 + nal.length, 0));
  const view = new DataView(out.buffer);
  let offset = 0;
  for (const nal of nals) {
    view.setUint32(offset, nal.length);
    out.set(nal, offset + 4);
    offset += 4 + nal.length;
  }
  return out;
}

// Builds the decoder config (codec string + avcC record) from scrcpy's SPS/PPS config packet
export function decoderConfigFrom(data: Uint8Array): VideoDecoderConfig | null {
  const nals = splitNals(data);
  const sps = nals.find((n) => (n[0] & 0x1f) === NAL_SPS);
  const pps = nals.find((n) => (n[0] & 0x1f) === NAL_PPS);
  if (!sps || !pps || sps.length < 4) return null;
  const avcc = new Uint8Array([
    1, sps[1], sps[2], sps[3], 0xff, 0xe1,
    sps.length >> 8, sps.length & 0xff, ...sps,
    1, pps.length >> 8, pps.length & 0xff, ...pps,
  ]);
  return {
    codec: `avc1.${hex2(sps[1])}${hex2(sps[2])}${hex2(sps[3])}`,
    description: avcc,
    optimizeForLatency: true,
  };
}
