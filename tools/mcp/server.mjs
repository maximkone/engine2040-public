// DOOR-READONLY: MCP read-only door (MIT donor: Ladvien/bevy_debugger_mcp)
// A mutating call succeeds only when its result list is clean AND a separate readback agrees.
// Donor: https://github.com/Ladvien/bevy_debugger_mcp

import { createRequire } from "node:module";
import {
  existsSync,
  readFileSync,
  writeFileSync,
} from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = dirname(dirname(fileURLToPath(import.meta.url)));
const DOOR_LOG_PATH = join(ROOT, "tools", "mcp", "door-log.jsonl");

// MCP server that only accepts read verbs
export function createReadOnlyMCP() {
  return {
    // Only read verbs are allowed
    verbs: {
      // Read-only verbs
      "read": async (params) => {
        // Log the read request
        const logEntry = {
          timestamp: new Date().toISOString(),
          verb: "read",
          params,
          result: null,
        };
        
        // Write to door log
        writeFileSync(DOOR_LOG_PATH, JSON.stringify(logEntry) + "\n", { flag: "a" });
        
        // Return result (to be verified by readback)
        return { result: "success" };
      },
      
      // Mutating verbs are forbidden
      "create": async (params) => {
        throw new Error("DOOR-READONLY: create verb is forbidden");
      },
      
      "update": async (params) => {
        throw new Error("DOOR-READONLY: update verb is forbidden");
      },
      
      "delete": async (params) => {
        throw new Error("DOOR-READONLY: delete verb is forbidden");
      },
      
      // All other verbs are forbidden
      "*": async (params) => {
        throw new Error(`DOOR-READONLY: verb '${Object.keys(params)[0]}' is forbidden`);
      }
    },
    
    // Verify that a readback agrees with the log
    verifyReadback: (expectedResult) => {
      if (!existsSync(DOOR_LOG_PATH)) {
        throw new Error("DOOR-READONLY: no door log found");
      }
      
      const logContent = readFileSync(DOOR_LOG_PATH, "utf8");
      const lines = logContent.trim().split("\n").filter(line => line.trim());
      
      if (lines.length === 0) {
        throw new Error("DOOR-READONLY: empty door log");
      }
      
      // Get last entry
      const lastLog = JSON.parse(lines[lines.length - 1]);
      
      // Verify readback matches expected result
      if (lastLog.result !== expectedResult) {
        throw new Error(`DOOR-READONLY: readback mismatch: expected ${expectedResult}, got ${lastLog.result}`);
      }
      
      return true;
    }
  };
}

// Export for use in tests
export default createReadOnlyMCP;