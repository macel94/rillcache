// Pinned node-redis client matrix harness for Rillcache readiness C7.
// Every case is reported exactly once; a thrown error fails that case only.
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { createClient, RESP_TYPES } from "redis";

const here = dirname(fileURLToPath(import.meta.url));
const lock = JSON.parse(readFileSync(join(here, "package-lock.json"), "utf8"));
const version = lock.packages["node_modules/redis"].version;

function parseArgs(argv) {
  const options = {};
  for (let index = 0; index < argv.length; index += 2) {
    const key = argv[index].replace(/^--/, "");
    options[key] = argv[index + 1];
  }
  return options;
}

const options = parseArgs(process.argv.slice(2));
for (const required of ["plain-port", "tls-port", "alt-tls-port", "named-user", "named-password",
  "limited-user", "limited-password", "default-password", "ca", "json-out"]) {
  if (!options[required]) {
    console.error(`missing --${required}`);
    process.exit(2);
  }
}

const ca = readFileSync(options.ca);
const records = [];
const key = (label) => `node-matrix:${label}`;

function client(port, { username, password, resp = 2, tls = false, retries = false } = {}) {
  const socket = { host: "127.0.0.1", port: Number(port), connectTimeout: 3000, socketTimeout: 3000 };
  if (tls) {
    socket.tls = true;
    socket.ca = [ca];
    // A TLS listener still needs a handshake; never let the client retry forever.
    socket.reconnectStrategy = retries ? (retry) => Math.min(retry * 50, 500) : false;
  } else {
    socket.reconnectStrategy = false;
  }
  return createClient({ socket, username, password, RESP: resp, database: 0 });
}

async function record(id, body) {
  try {
    const detail = await body();
    records.push({ id, status: "passed", detail: detail || "" });
  } catch (error) {
    records.push({ id, status: "failed", detail: String(error && error.message ? error.message : error).slice(0, 400) });
  }
}

function assert(condition, message) {
  if (!condition) throw new Error(message);
}

async function main() {
  const credentials = {
    username: options["named-user"],
    password: options["named-password"],
  };

  await record("handshake-auth", async () => {
    const c = client(options["plain-port"], credentials);
    await c.connect();
    assert((await c.ping()) === "PONG", "PING did not return PONG");
    await c.quit();
  });

  await record("acl-allowed", async () => {
    const c = client(options["plain-port"], credentials);
    await c.connect();
    await c.set(key("acl"), "value");
    assert((await c.get(key("acl"))) === "value", "named user could not round-trip its key");
    await c.quit();
  });

  await record("acl-denied-command", async () => {
    const c = client(options["plain-port"], {
      username: options["limited-user"], password: options["limited-password"],
    });
    await c.connect();
    try {
      await c.set(key("denied"), "value");
      throw new Error("limited user was allowed to SET");
    } catch (error) {
      assert(/NOPERM|no permissions/i.test(String(error.message)), `unexpected error: ${error.message}`);
    }
    await c.quit();
  });

  for (const [id, resp] of [["resp2-roundtrip", 2], ["resp3-roundtrip", 3]]) {
    await record(id, async () => {
      const c = client(options["plain-port"], { ...credentials, resp });
      await c.connect();
      assert((await c.ping()) === "PONG", `RESP${resp} PING failed`);
      // node-redis decodes bulk strings as UTF-8 by default; opt into Buffers
      // so binary payloads are compared as exact bytes.
      const bytes = c.withTypeMapping({ [RESP_TYPES.BLOB_STRING]: Buffer });
      const binary = Buffer.from([0x00, 0xff, 0x0d, 0x0a]);
      await c.set(key(`binary-${resp}`), binary);
      const read = await bytes.get(key(`binary-${resp}`));
      assert(Buffer.isBuffer(read), `RESP${resp} binary GET was not a Buffer`);
      assert(read.equals(binary), `RESP${resp} binary round-trip differed`);
      await c.set(key(`counter-${resp}`), "41");
      assert((await c.incr(key(`counter-${resp}`))) === 42, `RESP${resp} INCR differed`);
      await c.hSet(key(`hash-${resp}`), "field", "value");
      const hash = await c.hGetAll(key(`hash-${resp}`));
      if (resp === 3) {
        // The profile's bounded RESP3 subset emits arrays rather than maps (C6).
        assert(JSON.stringify(hash) === JSON.stringify(["field", "value"]),
          `RESP3 HGETALL shape differed: ${JSON.stringify(hash)}`);
        await c.quit();
        return "resp3-hgetall-flat-array: the profile's bounded RESP3 subset returns arrays, not maps (C6)";
      }
      assert(hash.field === "value", `RESP2 HGETALL differed: ${JSON.stringify(hash)}`);
      await c.quit();
      return "";
    });
  }

  await record("pipeline", async () => {
    const c = client(options["plain-port"], credentials);
    await c.connect();
    const replies = await c.multi()
      .set(key("p1"), "one").set(key("p2"), "two").mGet([key("p1"), key("p2")])
      .execAsPipeline();
    assert(JSON.stringify(replies) === JSON.stringify(["OK", "OK", ["one", "two"]]),
      `pipeline replies differed: ${JSON.stringify(replies)}`);
    await c.quit();
  });

  await record("transaction", async () => {
    const c = client(options["plain-port"], credentials);
    await c.connect();
    const replies = await c.multi().set(key("tx"), "value").get(key("tx")).exec();
    assert(JSON.stringify(replies) === JSON.stringify(["OK", "value"]),
      `transaction replies differed: ${JSON.stringify(replies)}`);
    assert((await c.get(key("tx"))) === "value", "transaction state was not committed");
    await c.quit();
  });

  await record("pubsub", async () => {
    const subscriber = client(options["plain-port"], credentials);
    const publisher = client(options["plain-port"], credentials);
    await Promise.all([subscriber.connect(), publisher.connect()]);
    const channel = key("channel");
    const received = new Promise((resolve) => subscriber.subscribe(channel, resolve));
    await new Promise((resolve) => setTimeout(resolve, 50));
    assert((await publisher.publish(channel, "payload")) === 1, "publish saw no subscriber");
    assert((await received) === "payload", "pub/sub payload differed");
    await Promise.all([subscriber.quit(), publisher.quit()]);
  });

  await record("error-wrong-type", async () => {
    const c = client(options["plain-port"], credentials);
    await c.connect();
    await c.set(key("wrong"), "value");
    try {
      await c.lPush(key("wrong"), "x");
      throw new Error("wrong-type write succeeded");
    } catch (error) {
      assert(/WRONGTYPE/i.test(String(error.message)), `unexpected error: ${error.message}`);
    }
    await c.quit();
  });

  await record("error-unknown-command", async () => {
    const c = client(options["plain-port"], credentials);
    await c.connect();
    try {
      await c.sendCommand(["NOSUCHCOMMAND"]);
      throw new Error("unknown command succeeded");
    } catch (error) {
      assert(/unknown command/i.test(String(error.message)), `unexpected error: ${error.message}`);
    }
    assert((await c.ping()) === "PONG", "connection was unusable after the error");
    await c.quit();
  });

  await record("error-auth-required", async () => {
    const c = client(options["plain-port"], {});
    await c.connect();
    try {
      await c.get(key("noauth"));
      throw new Error("unauthenticated read succeeded");
    } catch (error) {
      assert(/NOAUTH|authentication required/i.test(String(error.message)), `unexpected error: ${error.message}`);
    }
    await c.quit();
  });

  await record("reconnect", async () => {
    const c = client(options["plain-port"], credentials);
    await c.connect();
    await c.set(key("reconnect"), "value");
    await c.disconnect();
    await c.connect();
    assert((await c.get(key("reconnect"))) === "value", "value missing after reconnect");
    await c.quit();
  });

  await record("tls-positive", async () => {
    const c = client(options["tls-port"], { ...credentials, tls: true });
    await c.connect();
    assert((await c.ping()) === "PONG", "TLS PING failed");
    await c.set(key("tls"), "value");
    assert((await c.get(key("tls"))) === "value", "TLS round-trip differed");
    await c.quit();
  });

  await record("tls-wrong-identity", async () => {
    const c = client(options["alt-tls-port"], { ...credentials, tls: true });
    try {
      await c.connect();
      await c.ping();
      throw new Error("certificate without the target IP SAN was accepted");
    } catch (error) {
      const message = String(error.message);
      assert(!/was accepted/.test(message), message);
      assert(/cert|altname|hostname|self.signed/i.test(message), `unexpected TLS error: ${message}`);
    } finally {
      await c.disconnect().catch(() => {});
    }
  });

  const output = { client: "node-redis", version, protocols: [2, 3], cases: records };
  const { writeFileSync } = await import("node:fs");
  writeFileSync(options["json-out"], `${JSON.stringify(output, null, 2)}\n`);
  const failed = records.filter((case_) => case_.status !== "passed");
  console.log(`node-redis ${version}: ${records.length - failed.length}/${records.length} cases passed`);
  if (failed.length) {
    for (const case_ of failed) console.error(`FAILED ${case_.id}: ${case_.detail}`);
    process.exit(1);
  }
}

main().catch((error) => {
  console.error(error);
  process.exit(1);
});
