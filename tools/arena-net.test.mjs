// Run: node --test tools/arena-net.test.mjs
import assert from "node:assert/strict";
import test from "node:test";
import { joinLine, judge, parseArgs, receiptLine, relayLine, simLine } from "./arena-net.mjs";

const CODE = "E2040NET-v1-00000000000007f8-600-2-41a00000-duel";

test("the host's join lines give the guest's address and code", () => {
  assert.deepEqual(
    joinLine(`arena net: a guest on this PC types: arena.exe --scene arena --join 127.0.0.1:50123 ${CODE}`),
    { addr: "127.0.0.1:50123", code: CODE },
  );
  assert.deepEqual(
    joinLine(`arena net: the guest types: arena.exe --scene arena --join 127.0.0.1:7041 ${CODE}-rfedcba9876543210`),
    { addr: "127.0.0.1:7041", code: `${CODE}-rfedcba9876543210` },
  );
  assert.equal(joinLine(`arena net: a guest on the LAN types: arena.exe --scene arena --join 10.0.0.2:7043 ${CODE}`), null);
  assert.equal(joinLine("arena net: invite code E2040NET-v1-x"), null);
});

test("sim, receipt and relay lines parse; anything else is null", () => {
  assert.deepEqual(simLine("arena sim: ticks=600 hash=0123456789abcdef"), { ticks: 600, hash: "0123456789abcdef" });
  assert.equal(simLine("arena sim: ticks=600 hash=xyz"), null);
  assert.deepEqual(receiptLine("arena net: receipt role=host ticks=600 desync=none stalls=3"), {
    role: "host", ticks: "600", desync: "none", stalls: "3",
  });
  assert.equal(receiptLine("arena net: joined"), null);
  assert.equal(relayLine("net-relay: serving on 127.0.0.1:61000 (token 0xe2040a11ce000001)"), "127.0.0.1:61000");
});

const peer = (hash, code = 0, desync = "none") => ({ code, sim: { ticks: 600, hash }, receipt: { desync } });
const run = (name, round, host, guest = host) => ({ name, round, host: peer(host), guest: peer(guest) });

test("judge passes agreeing runs and names every broken check", () => {
  const green = [
    run("plain", 1, "a"), run("plain", 2, "a"),
    run("spear", 1, "b"), run("spear", 2, "b"),
    run("fault", 1, "a"), run("fault", 2, "a"),
    run("relay", 1, "a"), run("relay", 2, "a"),
  ];
  assert.deepEqual(judge(green, 600), []);
  const split = [run("plain", 1, "a", "c"), run("plain", 2, "a")];
  assert.match(judge(split, 600).join("\n"), /host a != guest c/);
  const drift = [run("plain", 1, "a"), run("plain", 2, "a"), run("fault", 1, "d"), run("fault", 2, "d")];
  assert.match(judge(drift, 600).join("\n"), /the network changed the match/);
  const flaky = [run("plain", 1, "a"), run("plain", 2, "e")];
  assert.match(judge(flaky, 600).join("\n"), /the runs disagree/);
  const same = [run("plain", 1, "a"), run("spear", 1, "a")];
  assert.match(judge(same, 600).join("\n"), /spear: same hash/);
  const dead = [{ name: "plain", round: 1, host: peer("a", 1), guest: { code: 2, sim: null, receipt: null } }];
  assert.match(judge(dead, 600).join("\n"), /host exited 1[\s\S]*guest printed no arena sim line/);
});

test("args default to the release exe and refuse junk", () => {
  assert.deepEqual(parseArgs([]), { exe: "target/release/arena.exe", relayExe: null, ticks: 600, out: "target/arena-net" });
  assert.equal(parseArgs(["--ticks", "300", "--exe", "x.exe"]).ticks, 300);
  assert.throws(() => parseArgs(["--ticks"]), /needs a value/);
  assert.throws(() => parseArgs(["--ticks", "10"]), /whole number/);
  assert.throws(() => parseArgs(["--nope", "1"]), /unknown flag/);
});
