/**
 * Run the generated custody recipes through Studio's actual patch engine.
 * Requires Node 24 and an installed, built Sapio Studio checkout (PR #89).
 *
 * From that Studio checkout:
 *   npm run build:desktop
 *   node --import tsx /path/to/sapio/contrib/build-a-vault/studio-check.ts \
 *     --studio "$PWD" --cli /path/to/sapio-cli \
 *     --workspace /path/to/module-cache --artifacts /path/to/generated
 *
 * Generate manifest.json, modules and recipes with this example's build script
 * first. This check runs local WASM and schema validation; it never signs,
 * binds, broadcasts, or connects to a Bitcoin node.
 */

import assert from "node:assert/strict";
import { access, mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { performance } from "node:perf_hooks";
import { pathToFileURL } from "node:url";
import { parseArgs } from "node:util";

interface ModuleRecord {
  name: string;
  key: string;
  path: string;
}

interface RecipeRecord {
  name: string;
  patch: string;
  artifact: string;
}

interface Manifest {
  modules: ModuleRecord[];
  recipes: RecipeRecord[];
}

async function main(): Promise<void> {
  const started = performance.now();
  const secondsSince = (time: number) =>
    ((performance.now() - time) / 1000).toFixed(1);
  const { values } = parseArgs({
    options: {
      studio: { type: "string" },
      cli: { type: "string" },
      workspace: { type: "string" },
      artifacts: { type: "string" },
    },
    strict: true,
  });
  for (const option of ["studio", "cli", "workspace", "artifacts"] as const) {
    assert(
      values[option],
      `Pass --${option}; see the command at the top of studio-check.ts.`,
    );
  }
  const studio = path.resolve(values.studio!);
  const cli = path.resolve(values.cli!);
  const workspace = path.resolve(values.workspace!);
  const artifacts = path.resolve(values.artifacts!);
  const schemaWorker = path.join(studio, "dist/desktop/schema-worker.cjs");
  await access(schemaWorker);
  const { createSapioBridge } = await import(
    pathToFileURL(path.join(studio, "desktop/bridge.ts")).href
  );
  const { parsePatch, runPatch } = await import(
    pathToFileURL(path.join(studio, "src/patching/engine.ts")).href
  );
  const manifest: Manifest = JSON.parse(
    await readFile(path.join(artifacts, "manifest.json"), "utf8"),
  );
  assert(Array.isArray(manifest.modules), "Manifest must list its modules.");
  assert.equal(
    manifest.modules.length,
    10,
    "Build-a-vault exports ten building blocks.",
  );
  assert(
    Array.isArray(manifest.recipes) && manifest.recipes.length > 0,
    "Manifest must list its recipes.",
  );
  assert.equal(
    new Set(manifest.modules.map((module) => module.key)).size,
    10,
    "Each building block must have its own module identity.",
  );

  const temporary = await mkdtemp(path.join(tmpdir(), "build-a-vault-studio-"));
  try {
    let selectedModule: string | null = null;
    const bridge = createSapioBridge({
      settings: async () => ({ cliPath: cli, workspace, runtimeConfig: "" }),
      temporaryDirectory: temporary,
      schemaWorker,
      selection: {
        module: async () => selectedModule,
        key: async () => null,
        evaluator: async () => null,
      },
    });
    const status = await bridge.status();
    assert(status.available, status.error ?? "Sapio CLI is unavailable.");
    const modules = [];
    for (const expected of manifest.modules) {
      const moduleStarted = performance.now();
      console.log(`Loading ${expected.name} and checking both schemas…`);
      assert.equal(typeof expected.path, "string");
      assert.match(expected.key, /^[0-9a-f]{64}$/u);
      selectedModule = path.resolve(artifacts, expected.path);
      const loaded = await bridge.modules.load();
      assert(loaded, `${expected.name}: module load returned no module.`);
      assert.equal(
        loaded.key,
        expected.key,
        `${expected.name}: WASM hash differs from its saved patches.`,
      );
      modules.push(loaded);
      for (const side of ["arguments", "returns"] as const) {
        // A null value may be invalid, but compiling the module's real
        // schema must succeed inside Studio's validation worker.
        await bridge.modules.validate({ key: loaded.key, side, value: null });
      }
      console.log(
        `${expected.name}: loaded exact hash and compiled both schemas in ${secondsSince(moduleStarted)}s.`,
      );
    }
    console.log(
      "Studio loaded all ten exact module hashes and compiled their input/output schemas.",
    );

    const runtime = {
      invoke: async (key: string, args: unknown) =>
        JSON.parse(
          await bridge.modules.call({ key, args: JSON.stringify(args) }),
        ),
      validate: (key: string, side: "arguments" | "returns", value: unknown) =>
        bridge.modules.validate({ key, side, value }),
    };
    const executedModules = new Set<string>();
    for (const recipe of manifest.recipes) {
      const recipeStarted = performance.now();
      console.log(`Executing ${recipe.name} through Studio's patch engine…`);
      const patch = parsePatch(
        await readFile(path.resolve(artifacts, recipe.patch), "utf8"),
      );
      const sources = new Set<string>();
      for (const connection of patch.connections)
        sources.add(connection.source);
      const terminals: string[] = [];
      for (const node of patch.nodes) {
        if (!sources.has(node.id)) terminals.push(node.id);
      }
      assert.equal(
        terminals.length,
        1,
        `${recipe.name}: recipe must have one terminal contract block.`,
      );
      const result: { output: unknown; executed: string[] } = await runPatch(
        patch,
        modules,
        terminals[0]!,
        patch.context,
        runtime,
      );
      const expected = JSON.parse(
        await readFile(path.resolve(artifacts, recipe.artifact), "utf8"),
      );
      assert.deepEqual(
        result.output,
        expected,
        `${recipe.name}: Studio result differs from the generated artifact.`,
      );
      const explanation = await bridge.explain({
        artifact: JSON.stringify(result.output),
      });
      assert(
        explanation.artifact.nodes.length > 0,
        `${recipe.name}: result has no inspectable contract.`,
      );
      for (const node of patch.nodes) {
        if (result.executed.includes(node.id))
          executedModules.add(node.moduleKey);
      }
      console.log(
        `${recipe.name}: ${result.executed.length} blocks, matching artifact, ${explanation.artifact.nodes.length} contract occurrences in ${secondsSince(recipeStarted)}s.`,
      );
    }
    for (const module of manifest.modules) {
      assert(
        executedModules.has(module.key),
        `${module.name}: no recipe executed this building block.`,
      );
    }
    console.log(
      `All ${manifest.recipes.length} recipes passed through Studio, covering every building block in ${secondsSince(started)}s.`,
    );
  } finally {
    await rm(temporary, { recursive: true, force: true });
  }
}

void main();
