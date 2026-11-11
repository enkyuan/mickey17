// Scenario inputs shared by the HTTP clients (tests), the browser recorder and the ground truth.
// Two runs per scenario with different inputs; each run also has its own PRNG seed so the
// server-issued ids (cart, rate, order, token, session, csrf) differ between runs.
import type { Address } from "./core";

export type ScenarioName = "success" | "validation-error" | "out-of-stock" | "declined";
export interface RunInputs {
    seed: number;
    sku: string;
    quantity: string;
    address: Address;
    rate: "Standard" | "Express";
    /** validation-error only */
    badQuantity?: string;
    badAddress?: Partial<Address>;
}

const A1: Address = { name: "Ada Quill", line1: "12 Larch Lane", city: "Portland", region: "OR", postalCode: "97201", country: "US" };
const A2: Address = { name: "Bo Tanaka", line1: "900 Harbor Way", city: "Austin", region: "TX", postalCode: "78701", country: "US" };

export const SCENARIOS: Record<ScenarioName, { description: string; runs: RunInputs[] }> = {
    success: {
        description: "catalog, cart, address, rates, test payment approved, confirmation, order-status read",
        runs: [
            { seed: 11, sku: "mug", quantity: "2", address: A1, rate: "Standard" },
            { seed: 12, sku: "tote", quantity: "3", address: A2, rate: "Express" },
        ],
    },
    "validation-error": {
        description: "bad quantity (422), then good; bad postal code (422), then good; then the success tail",
        runs: [
            { seed: 21, sku: "mug", quantity: "1", address: A1, rate: "Express", badQuantity: "0", badAddress: { postalCode: "ABC" } },
            { seed: 22, sku: "kettle", quantity: "2", address: A2, rate: "Standard", badQuantity: "abc", badAddress: { name: "", region: "Texas" } },
        ],
    },
    "out-of-stock": {
        description: "quantity above stock (409 out_of_stock); the run ends with an empty cart",
        runs: [
            { seed: 31, sku: "kettle", quantity: "3", address: A1, rate: "Standard" },
            { seed: 32, sku: "tote", quantity: "6", address: A2, rate: "Standard" },
        ],
    },
    declined: {
        description: "full flow, test payment method declines (402 card_declined); no order exists",
        runs: [
            { seed: 41, sku: "mug", quantity: "1", address: A1, rate: "Standard" },
            { seed: 42, sku: "tote", quantity: "2", address: A2, rate: "Express" },
        ],
    },
};

export const SCENARIO_NAMES = Object.keys(SCENARIOS) as ScenarioName[];
export const STACKS = ["react", "server-rendered", "svelte"] as const;
export type Stack = (typeof STACKS)[number];
