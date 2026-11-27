Object(function installTextGlobals(encodeUtf8) {
  const wellFormed = Function.call.bind(String.prototype.toWellFormed);
  const charCodeAt = Function.call.bind(
    Object.getOwnPropertyDescriptor(String.prototype, "charCodeAt").value,
  );
  const Uint8 = Uint8Array;
  const isView = ArrayBuffer.isView.bind(ArrayBuffer);
  const fromCharCode = String.fromCharCode;
  const getter = (prototype, key) => {
    const get = Reflect.get(Object.getOwnPropertyDescriptor(prototype, key) ?? {}, "get");
    return get && Function.call.bind(get);
  };
  const typed = Object.getPrototypeOf(Uint8.prototype);
  const typedTag = getter(typed, Symbol.toStringTag);
  const typedLength = getter(typed, "length");
  const typedBuffer = getter(typed, "buffer");
  const typedOffset = getter(typed, "byteOffset");
  const typedBytes = getter(typed, "byteLength");
  const viewBuffer = getter(DataView.prototype, "buffer");
  const viewOffset = getter(DataView.prototype, "byteOffset");
  const viewBytes = getter(DataView.prototype, "byteLength");
  const detached = getter(ArrayBuffer.prototype, "detached");
  const resizable = getter(ArrayBuffer.prototype, "resizable");
  const bufferBytes = getter(ArrayBuffer.prototype, "byteLength");
  const sharedBytes =
    typeof SharedArrayBuffer === "undefined"
      ? undefined
      : getter(SharedArrayBuffer.prototype, "byteLength");
  const growable =
    typeof SharedArrayBuffer === "undefined"
      ? undefined
      : getter(SharedArrayBuffer.prototype, "growable");
  const empty = new Uint8(0);

  function asBytes(input) {
    if (input === undefined) return empty;
    if (isView(input)) {
      const isTyped = typedTag(input) !== undefined;
      const buffer = (isTyped ? typedBuffer : viewBuffer)(input);
      requireFixedBuffer(buffer);
      if (isDetached(buffer)) return empty;
      const offset = (isTyped ? typedOffset : viewOffset)(input);
      const length = (isTyped ? typedBytes : viewBytes)(input);
      return new Uint8(buffer, offset, length);
    }
    let length;
    try {
      length = bufferBytes(input);
    } catch {
      length = sharedBytes?.(input);
    }
    if (length === undefined) throw new TypeError("Expected a buffer source");
    requireFixedBuffer(input);
    return isDetached(input) ? empty : new Uint8(input, 0, length);
  }

  function isDetached(buffer) {
    if (!detached) return false;
    try {
      return detached(buffer);
    } catch {
      return false;
    } // SharedArrayBuffer
  }

  function requireFixedBuffer(buffer) {
    let dynamic;
    try {
      dynamic = resizable?.(buffer);
    } catch {
      dynamic = growable?.(buffer);
    }
    if (dynamic) throw new TypeError("Expected a fixed-length buffer");
  }

  function dictionary(options) {
    if (options != null && typeof options !== "object" && typeof options !== "function") {
      throw new TypeError("Expected an options object");
    }
    return options;
  }

  class TextEncoder {
    #brand;
    get encoding() {
      return this.#brand ?? "utf-8";
    }
    encode(input = "") {
      if (!(#brand in this)) throw new TypeError("Invalid TextEncoder receiver");
      return encodeUtf8(wellFormed(`${input}`));
    }
    encodeInto(source, destination) {
      if (!(#brand in this)) throw new TypeError("Invalid TextEncoder receiver");
      if (arguments.length < 2) throw new TypeError("Expected source and destination");
      source = `${source}`;
      if (typedTag(destination) !== "Uint8Array") throw new TypeError("Expected Uint8Array");
      requireFixedBuffer(typedBuffer(destination));
      const capacity = typedLength(destination);
      let read = 0;
      let written = 0;
      while (read < source.length && written < capacity) {
        let scalar = charCodeAt(source, read);
        let width = 1;
        if (scalar >= 0xd800 && scalar <= 0xdfff) {
          const next = charCodeAt(source, read + 1);
          if (scalar <= 0xdbff && next >= 0xdc00 && next <= 0xdfff) {
            scalar = 0x10000 + ((scalar - 0xd800) << 10) + (next - 0xdc00);
            width = 2;
          } else {
            scalar = 0xfffd;
          }
        }
        const bytes = scalar < 0x80 ? 1 : scalar < 0x800 ? 2 : scalar < 0x10000 ? 3 : 4;
        if (written + bytes > capacity) break;
        if (bytes === 1) {
          destination[written++] = scalar;
        } else {
          const prefix = bytes === 2 ? 0xc0 : bytes === 3 ? 0xe0 : 0xf0;
          destination[written++] = prefix | (scalar >> (6 * (bytes - 1)));
          for (let shift = 6 * (bytes - 2); shift >= 0; shift -= 6) {
            destination[written++] = 0x80 | ((scalar >> shift) & 0x3f);
          }
        }
        read += width;
      }
      return { read, written };
    }
  }

  class TextDecoder {
    #fatal;
    #ignoreBOM;
    #stream = false;
    #seenBOM = false;
    #left = 0;
    #scalar = 0;
    #min = 0x80;
    #max = 0xbf;

    constructor(label = "utf-8", options) {
      label = `${label}`.replace(/^[\t\n\f\r ]+|[\t\n\f\r ]+$/g, "").toLowerCase();
      options = dictionary(options);
      const fatal = !!options?.fatal;
      const ignoreBOM = !!options?.ignoreBOM;
      if (
        ![
          "utf-8",
          "utf8",
          "unicode-1-1-utf-8",
          "unicode11utf8",
          "unicode20utf8",
          "x-unicode20utf8",
        ].includes(label)
      ) {
        throw new RangeError("Unsupported text encoding");
      }
      this.#fatal = fatal;
      this.#ignoreBOM = ignoreBOM;
    }
    get encoding() {
      if (!(#fatal in this)) throw new TypeError("Invalid TextDecoder receiver");
      return "utf-8";
    }
    get fatal() {
      return this.#fatal;
    }
    get ignoreBOM() {
      return this.#ignoreBOM;
    }

    #resetSequence() {
      this.#left = 0;
      this.#scalar = 0;
      this.#min = 0x80;
      this.#max = 0xbf;
    }
    #malformed() {
      if (this.#fatal) {
        this.#stream = false;
        throw new TypeError("Invalid UTF-8 data");
      }
      return 0xfffd;
    }
    decode() {
      if (!(#fatal in this)) throw new TypeError("Invalid TextDecoder receiver");
      const bytes = asBytes(arguments[0]);
      const stream = !!dictionary(arguments[1])?.stream;
      if (!this.#stream) {
        this.#resetSequence();
        this.#seenBOM = false;
      }
      this.#stream = stream;
      const parts = [];
      const units = [];
      const emit = scalar => {
        if (!this.#ignoreBOM && !this.#seenBOM) {
          this.#seenBOM = true;
          if (scalar === 0xfeff) return;
        }
        if (scalar <= 0xffff) units.push(scalar);
        else units.push(0xd800 + ((scalar - 0x10000) >> 10), 0xdc00 + ((scalar - 0x10000) & 0x3ff));
        if (units.length >= 4096) {
          parts.push(fromCharCode(...units));
          units.length = 0;
        }
      };
      // WHATWG UTF-8 decoder: keep only the unfinished scalar, and reconsume a byte
      // that fails a continuation check so it can start its own scalar or error.
      for (let i = 0, length = typedLength(bytes); i < length; i++) {
        const byte = bytes[i];
        if (this.#left !== 0) {
          if (byte < this.#min || byte > this.#max) {
            this.#resetSequence();
            emit(this.#malformed());
            i--;
            continue;
          }
          this.#min = 0x80;
          this.#max = 0xbf;
          this.#scalar = (this.#scalar << 6) | (byte & 0x3f);
          if (--this.#left === 0) emit(this.#scalar);
        } else if (byte <= 0x7f) {
          emit(byte);
        } else if (byte >= 0xc2 && byte <= 0xdf) {
          this.#left = 1;
          this.#scalar = byte & 0x1f;
        } else if (byte >= 0xe0 && byte <= 0xef) {
          this.#left = 2;
          this.#scalar = byte & 0x0f;
          if (byte === 0xe0) this.#min = 0xa0;
          if (byte === 0xed) this.#max = 0x9f;
        } else if (byte >= 0xf0 && byte <= 0xf4) {
          this.#left = 3;
          this.#scalar = byte & 0x07;
          if (byte === 0xf0) this.#min = 0x90;
          if (byte === 0xf4) this.#max = 0x8f;
        } else {
          emit(this.#malformed());
        }
      }
      if (!stream && this.#left !== 0) {
        this.#resetSequence();
        emit(this.#malformed());
      }
      const tail = fromCharCode(...units);
      if (parts.length === 0) return tail;
      parts.push(tail);
      return parts.join("");
    }
  }

  for (const constructor of [TextEncoder, TextDecoder]) {
    for (const key of Object.getOwnPropertyNames(constructor.prototype)) {
      if (key !== "constructor")
        Object.defineProperty(constructor.prototype, key, { enumerable: true });
    }
    Object.defineProperty(constructor.prototype, Symbol.toStringTag, {
      value: constructor.name,
      configurable: true,
    });
    Object.defineProperty(globalThis, constructor.name, {
      value: constructor,
      writable: true,
      configurable: true,
    });
  }
});
