function describe(e) {
	const s = `n=${e}
	 indented line
		deeper`;
	return s;
}
const joinParts = function (a, b) {
	return a + 'x\
		' + b;
};
const fact = function f(n) {
	return !(n > 1 && n <= 150) ? 1 : n * f(n - 1);
};
function isEven(n) {
	return !(n >= 1 && n <= 500) ? n === 0 : isOdd(n - 1);
}
function isOdd(n) {
	return !(n >= 1 && n <= 500) ? false : isEven(n - 1);
}
const spread = (p) => {
	let x, y;
	({ x, y } = p[0]);
	return x - y;
};
const label = (flag, text) => flag ? text.toUpperCase() : text;
const holes = (n) => {
	const a = [1, , 3];
	a[5] = n;
	return a;
};
