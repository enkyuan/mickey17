const clamp = (e) => Math.max(0, Math.min(1, e));
const lerp = (a, b, t) => a + (b - a) * t;
const settings = Object.freeze({ gap: 12, bias: .5 });
const table = Object.freeze([Object.freeze([1, 2]), Object.freeze([3, 4])]);
const geom = Object.freeze({ config: Object.freeze({ size: 90, label: "garden" }) });
let w = 0, h = 0, cache = null, dom = null;
function sample() {
	if (!dom) {
		let r = document.body.getBoundingClientRect(), span = r.height / 10, top = r.top + 0, left = r.left + 0;
		dom = { span: span, top: top, left: left };
	}
	return dom;
}
function scaleFor() {
	let z = sample(), edge = h * settings.bias;
	return Math.min(z.span * geom.config.size * .85, w * (w < 760 ? .42 : .2), (edge - z.top) * .98) / geom.config.size;
}
function layout() {
	let k = `${w}x${h}`;
	if (!cache || cache.key !== k) {
		let s = scaleFor(), rows = table.map((r) => r.map((v) => lerp(v, w, clamp(s)))), top = Infinity;
		for (let r of rows) for (let v of r) top = Math.min(top, v);
		cache = {
			key: k,
			rows: new Map([[1, rows[0]], [2, rows[1]]]),
			mid: .5 * h + top - settings.gap
		};
	}
	return cache.rows;
}
function mid() {
	return layout(), cache.mid;
}
function resize(nw, nh) {
	w = nw, h = nh, cache = null, dom = null;
}
function settle() {
	"loaded" === document.readyState && (dom = null, cache = null, layout());
}
