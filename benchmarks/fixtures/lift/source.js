const clamp = (e) => Math.max(0, Math.min(1, e));
let z = 0;
const lerp = (a, b, t) => a + (b - a) * t;
function ease(e) {
	let t = clamp(e);
	return lerp(0, 1, t * t);
}
const bad = (e) => e + z;
const writes = () => { z = 1; };
const usesThis = function () { return this; };
