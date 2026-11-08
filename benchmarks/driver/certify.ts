import { readFile, writeFile } from "node:fs/promises";
import { hash } from "../src/fingerprint";
import { calibrationEvidence } from "../src/calibration";
const [controlPath, mutationPath, browserPath, out] = Bun.argv.slice(2);
if (!controlPath || !mutationPath || !browserPath || !out)
    throw Error(
        "Usage: bun driver/certify.ts CONTROL_REPORT MUTATION_REPORT BROWSER_WRAPPER NEW_CERTIFICATE",
    );
const control = JSON.parse(await readFile(controlPath, "utf8")),
    mutations = JSON.parse(await readFile(mutationPath, "utf8"));
const result = calibrationEvidence(
    control,
    mutations,
    hash(await readFile(browserPath)),
);
await writeFile(out, JSON.stringify(result, null, 2), { flag: "wx" });
if (!result.passed)
    throw Error("Calibration did not qualify; see certificate gates");
console.log(out);
