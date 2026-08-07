import WebSocket from "ws";

const since = Math.floor(Date.now() / 1000) - 900;
let n = 0;
const ws = new WebSocket("ws://localhost:3000");

ws.on("open", () => {
  ws.send(
    JSON.stringify([
      "REQ",
      "obs",
      { kinds: [24200], since, limit: 100 },
    ]),
  );
  // also filter by p tag for owner
  ws.send(
    JSON.stringify([
      "REQ",
      "obs_p",
      {
        kinds: [24200],
        "#p": [
          "ecebe11c8a0e8c19a7c6fd4982cd1e51fcbf0bc225238641de5e5747acc65451",
        ],
        since,
        limit: 100,
      },
    ]),
  );
  setTimeout(() => {
    console.log("count", n);
    ws.close();
    process.exit(0);
  }, 5000);
});

ws.on("message", (data) => {
  const s = data.toString();
  if (!s.includes('"EVENT"')) {
    if (s.includes("EOSE") || s.includes("NOTICE") || s.includes("OK")) {
      console.log("ctrl", s.slice(0, 200));
    }
    return;
  }
  n += 1;
  const j = JSON.parse(s);
  const e = j[2];
  const tags = (e.tags || [])
    .filter((t) => t[0] === "p" || t[0] === "agent" || t[0] === "frame")
    .map((t) => t.join("="));
  console.log(
    "event",
    e.kind,
    e.pubkey?.slice(0, 12),
    tags.join(","),
    "clen",
    (e.content || "").length,
    "created",
    e.created_at,
  );
});

ws.on("error", (e) => {
  console.error(e);
  process.exit(1);
});
