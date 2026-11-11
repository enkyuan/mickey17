// A small JSON parser that keeps number text exactly as written (ids above 2^53 and `1.0`
// must not be rewritten by a float round trip) and refuses hostile nesting.

export type JVal =
    | { t: "obj"; entries: Map<string, JVal> }
    | { t: "arr"; items: JVal[] }
    | { t: "str"; v: string }
    | { t: "num"; raw: string }
    | { t: "bool"; v: boolean }
    | { t: "null" };

const MAX_DEPTH = 128;

class Parser {
    i = 0;
    constructor(readonly s: string) {}

    ws(): void {
        while (this.i < this.s.length && " \t\n\r".includes(this.s[this.i]!)) this.i++;
    }

    fail(): never {
        throw new SyntaxError(`invalid JSON at ${this.i}`);
    }

    value(depth: number): JVal {
        if (depth > MAX_DEPTH) this.fail();
        this.ws();
        const c = this.s[this.i];
        if (c === "{") return this.object(depth);
        if (c === "[") return this.array(depth);
        if (c === '"') return { t: "str", v: this.string() };
        if (c === "t" && this.s.startsWith("true", this.i)) return ((this.i += 4), { t: "bool", v: true });
        if (c === "f" && this.s.startsWith("false", this.i)) return ((this.i += 5), { t: "bool", v: false });
        if (c === "n" && this.s.startsWith("null", this.i)) return ((this.i += 4), { t: "null" });
        const m = /^-?(?:0|[1-9]\d*)(?:\.\d+)?(?:[eE][+-]?\d+)?/.exec(this.s.slice(this.i, this.i + 400));
        if (!m) this.fail();
        this.i += m[0].length;
        return { t: "num", raw: m[0] };
    }

    object(depth: number): JVal {
        this.i++;
        const entries = new Map<string, JVal>();
        this.ws();
        if (this.s[this.i] === "}") return ((this.i++), { t: "obj", entries });
        for (;;) {
            this.ws();
            if (this.s[this.i] !== '"') this.fail();
            const k = this.string();
            this.ws();
            if (this.s[this.i++] !== ":") this.fail();
            entries.set(k, this.value(depth + 1));
            this.ws();
            const c = this.s[this.i++];
            if (c === "}") return { t: "obj", entries };
            if (c !== ",") this.fail();
        }
    }

    array(depth: number): JVal {
        this.i++;
        const items: JVal[] = [];
        this.ws();
        if (this.s[this.i] === "]") return ((this.i++), { t: "arr", items });
        for (;;) {
            items.push(this.value(depth + 1));
            this.ws();
            const c = this.s[this.i++];
            if (c === "]") return { t: "arr", items };
            if (c !== ",") this.fail();
        }
    }

    string(): string {
        const start = this.i;
        this.i++;
        for (;;) {
            const c = this.s[this.i++];
            if (c === undefined) this.fail();
            if (c === "\\") this.i++;
            else if (c === '"') break;
            else if (c < " ") this.fail();
        }
        return JSON.parse(this.s.slice(start, this.i)) as string;
    }
}

/** Parse; throws SyntaxError on invalid input or nesting deeper than 128. */
export function parseJson(text: string): JVal {
    const p = new Parser(text);
    const v = p.value(0);
    p.ws();
    if (p.i !== text.length) p.fail();
    return v;
}

export function tryParseJson(text: string): JVal | null {
    try {
        return parseJson(text);
    } catch {
        return null;
    }
}

/** Plain-object view for the small trusted structures (HAR envelope, trace). */
export function asObject(v: JVal | undefined): Map<string, JVal> | null {
    return v && v.t === "obj" ? v.entries : null;
}

export function asString(v: JVal | undefined): string | null {
    return v && v.t === "str" ? v.v : null;
}

export function asArray(v: JVal | undefined): JVal[] | null {
    return v && v.t === "arr" ? v.items : null;
}

export function asNumber(v: JVal | undefined): number | null {
    return v && v.t === "num" ? Number(v.raw) : null;
}
