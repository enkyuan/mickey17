// Reproduces the `vite dev` clash between React's development JSX runtime and a
// production-built renderer that shares its internals, using real React in a DOM.
// A recovered react-three-fiber reconciler assigns `{ getCacheForType, cacheSignal }`
// to ReactSharedInternals.A while it renders; React's dev `jsx()` then calls
// `A.getOwner()`. Run with NODE_ENV=development:
//
//   NODE_ENV=development bun tests/compat-dev-react.ts <export dir> [guard]
//
// Prints {"errors":[...],"html":"..."}; with `guard` the template's
// installOwnerGuard is applied first (as router.tsx does in dev).
import { Window } from "happy-dom";
import { resolve } from "node:path";

const dir = resolve(process.argv[2] ?? "");
const window = new Window();
Object.assign(globalThis, { window, document: window.document, navigator: window.navigator, IS_REACT_ACT_ENVIRONMENT: false });
const React = await import(dir + "/node_modules/react/index.js");
const { createRoot } = await import(dir + "/node_modules/react-dom/client.js");
const { jsx } = await import(dir + "/node_modules/react/jsx-runtime.js");
const key = "__CLIENT_INTERNALS_DO_NOT_USE_OR_WARN_USERS_THEY_CANNOT_UPGRADE";
const internals = (React[key] ?? React.default?.[key]) as { A: unknown };
if (process.argv[3] === "guard") {
    const { installOwnerGuard } = await import("../../src/export_template/src/lib/policy");
    installOwnerGuard(internals);
}
const foreign = { getCacheForType: () => ({}), cacheSignal: () => null };
const Child = () => jsx("span", { children: "ok" });
function Foreign() {
    const previous = internals.A;
    internals.A = foreign;
    try {
        return jsx("div", { children: jsx(Child, {}) });
    } finally {
        internals.A = previous;
    }
}
const errors: string[] = [];
const host = window.document.createElement("div");
window.document.body.append(host);
createRoot(host as unknown as Element, {
    onUncaughtError: (e: unknown) => errors.push(String(e)),
    onCaughtError: (e: unknown) => errors.push(String(e)),
}).render(jsx(Foreign, {}));
await new Promise((r) => setTimeout(r, 50));
console.log(JSON.stringify({ version: React.version, node_env: process.env.NODE_ENV, errors, html: host.innerHTML }));
process.exit(0);
