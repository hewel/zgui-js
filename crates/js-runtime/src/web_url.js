Object(
  function installUrlGlobals(
    parseUrl,
    canParseUrl,
    getUrl,
    setUrl,
    parseParams,
    serializeParams,
    component,
  ) {
    const wellFormed = Function.call.bind(String.prototype.toWellFormed);
    const usv = value => wellFormed(`${value}`);
    const objectLike = value =>
      value !== null && (typeof value === "object" || typeof value === "function");
    const requireArgs = (actual, required) => {
      if (actual < required) throw new TypeError(`Expected at least ${required} argument(s)`);
    };
    const readQuery = value => parseParams(value.startsWith("?") ? value.slice(1) : value);
    const nativeUrl = Symbol();
    let pairsOf;
    let replaceParams;
    let bindParams;

    class SearchParamsIterator {
      #params;
      #kind;
      #index = 0;

      constructor(params, kind) {
        this.#params = params;
        this.#kind = kind;
      }

      next() {
        const pairs = pairsOf(this.#params);
        if (this.#index >= pairs.length) return { value: undefined, done: true };
        const [name, value] = pairs[this.#index++];
        return {
          value: this.#kind === 0 ? [name, value] : this.#kind === 1 ? name : value,
          done: false,
        };
      }
    }

    Object.setPrototypeOf(
      SearchParamsIterator.prototype,
      Object.getPrototypeOf(Object.getPrototypeOf([][Symbol.iterator]())),
    );
    Reflect.deleteProperty(SearchParamsIterator.prototype, "constructor");
    Object.defineProperty(SearchParamsIterator.prototype, Symbol.toStringTag, {
      value: "URLSearchParams Iterator",
      configurable: true,
    });

    class URLSearchParams {
      #pairs;
      #update;

      static {
        pairsOf = params => params.#pairs;
        replaceParams = (params, query) => {
          params.#pairs = readQuery(query);
        };
        bindParams = (params, update) => {
          params.#update = update;
        };
      }

      constructor(init = "") {
        if (!objectLike(init)) {
          this.#pairs = readQuery(usv(init));
          return;
        }
        this.#pairs = [];
        const iterator = init[Symbol.iterator];
        if (iterator != null) {
          for (const entry of { [Symbol.iterator]: () => Reflect.apply(iterator, init, []) }) {
            if (!objectLike(entry)) throw new TypeError("Expected an iterable pair");
            const pairIterator = entry[Symbol.iterator];
            if (typeof pairIterator !== "function")
              throw new TypeError("Expected an iterable pair");
            const pair = Array.from(
              { [Symbol.iterator]: () => Reflect.apply(pairIterator, entry, []) },
              usv,
            );
            if (pair.length !== 2) throw new TypeError("Expected a pair of length 2");
            this.#pairs.push(pair);
          }
        } else {
          // USVString conversion can collapse distinct keys with lone surrogates.
          const record = new Map();
          for (const key of Reflect.ownKeys(init)) {
            if (Object.getOwnPropertyDescriptor(init, key)?.enumerable) {
              record.set(usv(key), usv(init[key]));
            }
          }
          this.#pairs = Array.from(record);
        }
      }

      get size() {
        return this.#pairs.length;
      }
      append(name, value) {
        requireArgs(arguments.length, 2);
        const pair = [usv(name), usv(value)];
        this.#pairs.push(pair);
        this.#changed();
      }
      delete(name) {
        requireArgs(arguments.length, 1);
        name = usv(name);
        let value = arguments[1];
        if (value !== undefined) value = usv(value);
        this.#pairs = this.#pairs.filter(
          pair => pair[0] !== name || (value !== undefined && pair[1] !== value),
        );
        this.#changed();
      }
      get(name) {
        requireArgs(arguments.length, 1);
        name = usv(name);
        return this.#pairs.find(pair => pair[0] === name)?.[1] ?? null;
      }
      getAll(name) {
        requireArgs(arguments.length, 1);
        name = usv(name);
        return this.#pairs.filter(pair => pair[0] === name).map(pair => pair[1]);
      }
      has(name) {
        requireArgs(arguments.length, 1);
        name = usv(name);
        let value = arguments[1];
        if (value !== undefined) value = usv(value);
        return this.#pairs.some(
          pair => pair[0] === name && (value === undefined || pair[1] === value),
        );
      }
      set(name, value) {
        requireArgs(arguments.length, 2);
        name = usv(name);
        value = usv(value);
        let found = false;
        this.#pairs = this.#pairs.filter(pair => {
          if (pair[0] !== name) return true;
          if (found) return false;
          found = true;
          pair[1] = value;
          return true;
        });
        if (!found) this.#pairs.push([name, value]);
        this.#changed();
      }
      sort() {
        this.#pairs.sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0));
        this.#changed();
      }
      #changed() {
        if (this.#update) this.#update(serializeParams(this.#pairs));
      }
      #iterator(kind) {
        return new SearchParamsIterator(this, kind);
      }
      entries() {
        return this.#iterator(0);
      }
      keys() {
        return this.#iterator(1);
      }
      values() {
        return this.#iterator(2);
      }
      forEach(callback) {
        if (!(#pairs in this)) throw new TypeError("Invalid URLSearchParams receiver");
        if (typeof callback !== "function") throw new TypeError("Expected a callback");
        const thisArg = arguments[1];
        for (let i = 0; i < this.#pairs.length; i++) {
          const [name, value] = this.#pairs[i];
          Reflect.apply(callback, thisArg, [value, name, this]);
        }
      }
      toString() {
        return serializeParams(this.#pairs);
      }
    }

    class URL {
      #url;
      #params;
      static #parse(input, base) {
        input = usv(input);
        return base === undefined ? parseUrl(input) : parseUrl(input, usv(base));
      }
      constructor(input) {
        requireArgs(arguments.length, 1);
        const base = arguments[1];
        this.#url = base === nativeUrl ? input : URL.#parse(input, base);
        if (!this.#url) throw new TypeError("Invalid URL");
      }
      static canParse(input) {
        requireArgs(arguments.length, 1);
        const base = arguments[1];
        input = usv(input);
        return base === undefined ? canParseUrl(input) : canParseUrl(input, usv(base));
      }
      static parse(input) {
        requireArgs(arguments.length, 1);
        const parsed = URL.#parse(input, arguments[1]);
        return parsed ? new URL(parsed, nativeUrl) : null;
      }
      get href() {
        return getUrl(this.#url, component.href);
      }
      set href(value) {
        this.#setQueryComponent(component.href, value);
      }
      get origin() {
        return getUrl(this.#url, component.origin);
      }
      get protocol() {
        return getUrl(this.#url, component.protocol);
      }
      set protocol(value) {
        setUrl(this.#url, component.protocol, usv(value));
      }
      get username() {
        return getUrl(this.#url, component.username);
      }
      set username(value) {
        setUrl(this.#url, component.username, usv(value));
      }
      get password() {
        return getUrl(this.#url, component.password);
      }
      set password(value) {
        setUrl(this.#url, component.password, usv(value));
      }
      get host() {
        return getUrl(this.#url, component.host);
      }
      set host(value) {
        setUrl(this.#url, component.host, usv(value));
      }
      get hostname() {
        return getUrl(this.#url, component.hostname);
      }
      set hostname(value) {
        setUrl(this.#url, component.hostname, usv(value));
      }
      get port() {
        return getUrl(this.#url, component.port);
      }
      set port(value) {
        setUrl(this.#url, component.port, usv(value));
      }
      get pathname() {
        return getUrl(this.#url, component.pathname);
      }
      set pathname(value) {
        setUrl(this.#url, component.pathname, usv(value));
      }
      get search() {
        return getUrl(this.#url, component.search);
      }
      set search(value) {
        this.#setQueryComponent(component.search, value);
      }
      get hash() {
        return getUrl(this.#url, component.hash);
      }
      set hash(value) {
        setUrl(this.#url, component.hash, usv(value));
      }
      #setQueryComponent(key, value) {
        const url = this.#url;
        setUrl(url, key, usv(value));
        if (this.#params) replaceParams(this.#params, getUrl(url, component.search));
      }
      get searchParams() {
        if (!this.#params) {
          const url = this.#url;
          this.#params = new URLSearchParams(getUrl(url, component.search));
          bindParams(this.#params, query => setUrl(url, component.search, query));
        }
        return this.#params;
      }
      toString() {
        return getUrl(this.#url, component.href);
      }
      toJSON() {
        return getUrl(this.#url, component.href);
      }
    }

    Object.defineProperty(URLSearchParams.prototype, Symbol.iterator, {
      value: Object.getOwnPropertyDescriptor(URLSearchParams.prototype, "entries").value,
      writable: true,
      configurable: true,
    });
    for (const constructor of [URL, URLSearchParams]) {
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
    for (const key of ["canParse", "parse"]) Object.defineProperty(URL, key, { enumerable: true });
  },
);
