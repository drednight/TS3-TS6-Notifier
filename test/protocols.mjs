/**
 * Protocol test harness: fake TS3 ClientQuery (TCP 25739) and fake TS6
 * Remote Apps (WS 5999) servers + assertions against the real connectors.
 * Run: node test/protocols.mjs  (with the notifier running separately is NOT
 * needed — this drives the connectors directly.)
 */
import net from 'node:net';
import { WebSocketServer } from 'ws';
import { Ts3Connector } from '../server/connectors/ts3.ts';
import { Ts6Connector } from '../server/connectors/ts6.ts';

const results = [];
function check(name, cond, extra = '') {
  results.push({ name, ok: !!cond, extra });
  console.log(`${cond ? 'PASS' : 'FAIL'}  ${name}${extra ? ' — ' + extra : ''}`);
}

/* ---------------- fake TS3 ClientQuery ---------------- */

const ts3Clients = [
  { clid: 1, cid: 10, client_nickname: 'Alice', client_flag_talking: 1, client_input_muted: 0, client_input_hardware: 1, client_output_muted: 0, client_output_hardware: 1, client_away: 0, client_type: 0, client_is_channel_commander: 1, client_is_priority_speaker: 0, client_servergroups: '6' },
  { clid: 2, cid: 10, client_nickname: 'Борис', client_flag_talking: 0, client_input_muted: 1, client_input_hardware: 1, client_output_muted: 0, client_output_hardware: 1, client_away: 1, client_away_message: 'brb', client_type: 0, client_is_channel_commander: 0, client_is_priority_speaker: 1, client_servergroups: '12' },
  { clid: 3, cid: 11, client_nickname: 'Other', client_type: 0 },
  { clid: 4, cid: 10, client_nickname: 'QueryBot', client_type: 1 },
];

function esc(v) {
  return String(v).replace(/([\\/\s|])/g, (m) => (m === ' ' ? '\\s' : m === '|' ? '\\p' : m === '/' ? '\\/' : '\\\\'));
}

function fields(item) {
  return Object.entries(item).map(([k, v]) => `${k}=${esc(v)}`).join(' ');
}

function ts3Response(cmd) {
  if (cmd.startsWith('auth ')) return 'error id=0 msg=ok';
  if (cmd.startsWith('clientnotifyregister')) return 'error id=0 msg=ok';
  if (cmd.startsWith('serverconnectionhandlerlist')) return 'schandlerid=1|schandlerid=2\nerror id=0 msg=ok';
  if (cmd.startsWith('use ')) return 'error id=0 msg=ok';
  if (cmd.startsWith('servergrouplist')) {
    return (
      [fields({ sgid: 6, name: 'Warriors', iconid: 12345 }), fields({ sgid: 12, name: 'Маги', iconid: 54321 })].join('|') +
      '\nerror id=0 msg=ok'
    );
  }
  if (cmd.startsWith('whoami')) return fields({ clid: 1, cid: 10, client_nickname: 'Alice' }) + '\nerror id=0 msg=ok';
  if (cmd.startsWith('clientvariable') && cmd.includes('client_input_hardware') && !cmd.includes('client_flag_avatar')) {
    return fields({ clid: 1, client_input_hardware: 1 }) + '\nerror id=0 msg=ok';
  }
  if (cmd.startsWith('servervariable')) return fields({ virtualserver_name: 'Test Server' }) + '\nerror id=0 msg=ok';
  if (cmd.startsWith('channellist')) {
    return [fields({ cid: 10, pid: 0, channel_name: 'Main Lounge', channel_order: 0 }), fields({ cid: 11, pid: 0, channel_name: 'AFK', channel_order: 1 })].join('|') + '\nerror id=0 msg=ok';
  }
  if (cmd.startsWith('clientlist')) {
    return ts3Clients.map(fields).join('|') + '\nerror id=0 msg=ok';
  }
  if (cmd.startsWith('clientvariable')) {
    // avatar batch: clid=N client_flag_avatar|...
    const out = cmd
      .slice('clientvariable '.length)
      .split('|')
      .map((blk) => {
        const m = /clid=(\d+)/.exec(blk);
        return m ? fields({ clid: Number(m[1]), client_flag_avatar: 'deadbeef' }) : '';
      })
      .filter(Boolean);
    return out.join('|') + '\nerror id=0 msg=ok';
  }
  return 'error id=0 msg=ok';
}

const ts3Server = net.createServer((socket) => {
  socket.write('TS3 Client\n\nselected schandlerid=1\n');
  globalThis.ts3LastSocket = socket;
  let buf = '';
  socket.on('data', (d) => {
    buf += d.toString();
    let idx;
    while ((idx = buf.indexOf('\n')) >= 0) {
      const cmd = buf.slice(0, idx).replace(/\r$/, '');
      buf = buf.slice(idx + 1);
      if (cmd === 'quit') return;
      socket.write(ts3Response(cmd) + '\n');
    }
  });
});

/* ---------------- fake TS6 Remote Apps ---------------- */

const ts6Users = [
  { id: 100, channelId: 5, properties: { nickname: 'Ruslan', flagTalking: '1', inputMuted: false, outputMuted: false, away: false, isChannelCommander: true, flagAvatar: 'abc123', serverGroups: '6' } },
  { id: 101, channelId: 5, properties: { nickname: 'Elena', flagTalking: '0', inputMuted: true, outputMuted: true, away: true, awayMessage: 'tea', isPrioritySpeaker: true } },
  { id: 102, channelId: 9, properties: { nickname: 'Elsewhere', flagTalking: '1' } },
  { id: 1, channelId: 5, properties: { nickname: 'Me', flagTalking: '0' } },
];

function ts6AuthDump() {
  return {
    type: 'auth',
    status: { code: 0, message: 'ok' },
    payload: {
      apiKey: 'test-key-123',
      currentConnectionId: 7,
      connections: [
        {
          id: 7,
          clientId: 1,
          properties: { name: 'TS6 Test Server' },
          channelInfos: {
            rootChannels: [
              { id: 5, parentId: 0, properties: { name: 'General', order: 0 } },
              { id: 9, parentId: 0, properties: { name: 'AFK', order: 1 } },
              { id: 88, parentId: 0, properties: { name: 'DiffLab', order: 2 } },
            ],
            subChannels: {},
          },
          clientInfos: ts6Users.map((u) => ({ id: u.id, channelId: u.channelId, properties: u.properties })),
        },
      ],
    },
  };
}

const ts6Wss = new WebSocketServer({ port: 5999 });
ts6Wss.on('connection', (ws) => {
  globalThis.ts6Sock = ws;
  ws.on('message', (raw) => {
    const msg = JSON.parse(raw.toString());
    if (msg.type === 'auth') {
      ws.send(JSON.stringify(ts6AuthDump()));
      // wrapped event + top-level events after a beat
      setTimeout(() => {
        ws.send(JSON.stringify({ type: 'proto.ServerEvent', payload: { connectionId: 7, talkStatusChanged: { clientId: 101, status: 1 } } }));
        // whisper: status ≥ 2
        ws.send(JSON.stringify({ type: 'talkStatusChanged', payload: { connectionId: 7, clientId: 100, status: 2 } }));
        // chat messages (defensive name parsing)
        ws.send(JSON.stringify({ type: 'message', payload: { connectionId: 7, msg: 'privet', invokerId: 101, invokerName: 'Elena', targetMode: 2 } }));
        ws.send(JSON.stringify({ type: 'proto.ServerEvent', payload: { connectionId: 7, chatMessage: { text: 'wrapped hi', senderName: 'Ruslan', senderId: 100, targetMode: 3 } } }));
        // sender as a nested object with a decoy message `id` — the sender id must win
        ws.send(JSON.stringify({ type: 'message', payload: { connectionId: 7, message: { id: 205, sender: { id: 101, name: 'Sigma' }, text: 'from sender object' } } }));
        ws.send(JSON.stringify({ type: 'message', payload: { connectionId: 7, message: { id: 206, senderId: 100, text: 'decoy id beaten' } } }));
        ws.send(JSON.stringify({ type: 'clientMoved', payload: { connectionId: 7, clientId: 102, newChannelId: 5, oldChannelId: 9 } }));
        ws.send(JSON.stringify({ type: 'clientSelfPropertyUpdated', payload: { connectionId: 7, flag: 'inputMuted', newValue: true } }));
        ws.send(JSON.stringify({ type: 'clientPropertiesUpdated', payload: { connectionId: 7, clientId: 100, properties: { nickname: 'Ruslan2' } } }));
      }, 120);
    }
  });
});

/* ---------------- run the real connectors ---------------- */

await new Promise((r) => ts3Server.listen(25739, '127.0.0.1', r));

const ts3States = [];
const c3 = new Ts3Connector('127.0.0.1', 25739, 'my-key');
c3.on('state', (s) => ts3States.push(s));
c3.start();

const ts6States = [];
let savedKey = '';
const c6 = new Ts6Connector('127.0.0.1', 5999, '', (k) => (savedKey = k));
c6.on('state', (s) => ts6States.push(s));
c6.start();

await new Promise((r) => setTimeout(r, 2500));

/* ---- TS3 assertions ---- */
const last3 = ts3States[ts3States.length - 1];
check('ts3: reached live state', last3?.status === 'live', `status=${last3?.status}`);
check('ts3: server name', last3?.serverName === 'Test Server', last3?.serverName);
check('ts3: channel name unescaped', last3?.channelName === 'Main Lounge', JSON.stringify(last3?.channelName));
check('ts3: only own-channel users', last3?.users.length === 3, `users=${last3?.users.map((u) => u.nickname).join(',')}`);
check('ts3: talking flag', last3?.users.find((u) => u.nickname === 'Alice')?.talking === true);
check('ts3: input muted + away', (() => {
  const b = last3?.users.find((u) => u.nickname === 'Борис');
  return b?.inputMuted === true && b?.away === true && b?.awayMessage === 'brb';
})());
check('ts3: query bot marked', last3?.users.find((u) => u.nickname === 'QueryBot')?.isQuery === true);
check('ts3: self marked', last3?.users.find((u) => u.nickname === 'Alice')?.isSelf === true);
check('ts3: commander + priority', (() => {
  const a = last3?.users.find((u) => u.nickname === 'Alice');
  const b = last3?.users.find((u) => u.nickname === 'Борис');
  return a?.channelCommander === true && b?.prioritySpeaker === true;
})());
check('ts3: avatar hash', last3?.users.find((u) => u.nickname === 'Alice')?.avatarHash === 'deadbeef');
check('ts3: server groups fetched', (() => {
  const g = last3?.groups || [];
  return g.length === 2 && g[0].id === 6 && g[0].iconId === 12345 && g[1].name === 'Маги';
})(), JSON.stringify(last3?.groups));

/* live notifications: chat + whisper */
globalThis.ts3LastSocket?.write('notifytextmessage targetmode=2 msg=hello\\schat schandlerid=1 invokerid=2 invokername=Борис\n');
globalThis.ts3LastSocket?.write('notifytalkstatuschange targetmode=1 status=1 clid=2 schandlerid=1\n');
await new Promise((r) => setTimeout(r, 500));
const last3b = ts3States[ts3States.length - 1];
check('ts3: chat message captured', (() => {
  const m = last3b?.chat || [];
  return m.length === 1 && m[0].text === 'hello chat' && m[0].nickname === 'Борис' && m[0].kind === 'channel';
})(), JSON.stringify(last3b?.chat));
check('ts3: whisper detected from targetmode', last3b?.users.find((u) => u.nickname === 'Борис')?.whispering === true);

/* duplicate delivery (own-message echo) must collapse into one entry */
globalThis.ts3LastSocket?.write('notifytextmessage targetmode=3 msg=dup\\stest schandlerid=1 invokerid=3 invokername=Vasya\n');
globalThis.ts3LastSocket?.write('notifytextmessage targetmode=3 msg=dup\\stest schandlerid=1 invokerid=3 invokername=Vasya\n');
await new Promise((r) => setTimeout(r, 400));
const last3c = ts3States[ts3States.length - 1];
check('ts3: duplicate delivery merged into one message', (() => {
  const m = (last3c?.chat || []).filter((x) => x.text === 'dup test');
  return m.length === 1 && m[0].nickname === 'Vasya' && m[0].kind === 'server';
})(), JSON.stringify(last3c?.chat));

/* chat from another server tab must not be dropped */
globalThis.ts3LastSocket?.write('notifytextmessage targetmode=2 msg=other\\stab schandlerid=2 invokerid=4 invokername=Oleg\n');
await new Promise((r) => setTimeout(r, 400));
const last3d = ts3States[ts3States.length - 1];
check('ts3: chat from another server tab accepted', (last3d?.chat || []).some((x) => x.text === 'other tab' && x.nickname === 'Oleg'),
  JSON.stringify(last3d?.chat));

/* GUI server-tab switch must move the tracked handler */
globalThis.ts3LastSocket?.write('notifycurrentserverconnectionchanged schandlerid=2\n');
await new Promise((r) => setTimeout(r, 800));
check('ts3: follows notifycurrentserverconnectionchanged', c3.activeSchandlerId === 2, `active=${c3.activeSchandlerId}`);
check('ts3: still live after tab switch', ts3States[ts3States.length - 1]?.status === 'live', ts3States[ts3States.length - 1]?.status);

/* event feed: join / leave / poke */
globalThis.ts3LastSocket?.write('notifycliententerview clid=5 ctid=1 client_nickname=Гоша client_type=0 schandlerid=2\n');
globalThis.ts3LastSocket?.write('notifyclientpoke invokerid=2 invokername=Борис msg=privet schandlerid=2\n');
globalThis.ts3LastSocket?.write('notifyclientleftview clid=5 client_nickname=Гоша schandlerid=2\n');
await new Promise((r) => setTimeout(r, 400));
const last3e = ts3States[ts3States.length - 1];
check('ts3: join feed row rendered', (last3e?.chat || []).some((x) => x.kind === 'system' && x.event === 'join' && x.nickname === 'Гоша'),
  JSON.stringify(last3e?.chat));
check('ts3: leave feed row rendered (events never merge)', (last3e?.chat || []).some((x) => x.kind === 'system' && x.event === 'leave' && x.nickname === 'Гоша'),
  JSON.stringify(last3e?.chat));
check('ts3: poke row rendered', (last3e?.chat || []).some((x) => x.kind === 'poke' && x.nickname === 'Борис' && x.detail === 'privet'),
  JSON.stringify(last3e?.chat));

/* ---- TS6 assertions ---- */
const last6 = ts6States[ts6States.length - 1];
check('ts6: reached live state', last6?.status === 'live', `status=${last6?.status}`);
check('ts6: api key saved', savedKey === 'test-key-123', savedKey);
check('ts6: server name', last6?.serverName === 'TS6 Test Server');
check('ts6: channel name', last6?.channelName === 'General', last6?.channelName);
check('ts6: own channel users only', last6?.users.length === 4, `users=${last6?.users.map((u) => u.nickname).join(',')}`);
check('ts6: moved user joined channel', last6?.users.some((u) => u.nickname === 'Elsewhere'));
check('ts6: wrapped talk event applied', last6?.users.find((u) => u.nickname === 'Elena')?.talking === true);
check('ts6: self inputMuted event applied', last6?.users.find((u) => u.nickname === 'Me')?.inputMuted === true);
check('ts6: nickname update applied', last6?.users.some((u) => u.nickname === 'Ruslan2'));
check('ts6: away + priority flags', (() => {
  const e = last6?.users.find((u) => u.nickname === 'Elena');
  return e?.away === true && e?.awayMessage === 'tea' && e?.prioritySpeaker === true && e?.outputMuted === true;
})());
check('ts6: commander flag', last6?.users.find((u) => u.nickname === 'Ruslan2')?.channelCommander === true);
check('ts6: whisper flag from status≥2', last6?.users.find((u) => u.nickname === 'Ruslan2')?.whispering === true);
check('ts6: chat message parsed', (() => {
  const m = last6?.chat || [];
  return m.some((x) => x.text === 'privet' && x.nickname === 'Elena' && x.kind === 'channel');
})(), JSON.stringify(last6?.chat));
check('ts6: wrapped chat parsed', (() => {
  const m = last6?.chat || [];
  return m.some((x) => x.text === 'wrapped hi' && x.nickname === 'Ruslan' && x.kind === 'server');
})(), JSON.stringify(last6?.chat));
check('ts6: sender-object chat resolved (message id must not leak)', (() => {
  const m = last6?.chat || [];
  return m.some((x) => x.text === 'from sender object' && x.nickname === 'Sigma' && x.userId === '101');
})(), JSON.stringify(last6?.chat));
check('ts6: senderId beats generic message id', (() => {
  const m = last6?.chat || [];
  return m.some((x) => x.text === 'decoy id beaten' && x.userId === '100');
})(), JSON.stringify(last6?.chat));

/* event feed on TS6: clientConnected */
globalThis.ts6Sock?.send(JSON.stringify({ type: 'clientConnected', payload: { connectionId: 7, clientId: 103, channelId: 2, properties: { nickname: 'Новичок' } } }));
await new Promise((r) => setTimeout(r, 300));
check('ts6: join feed row rendered', (() => {
  const m = ts6States[ts6States.length - 1]?.chat || [];
  return m.some((x) => x.kind === 'system' && x.event === 'join' && x.nickname === 'Новичок');
})(), JSON.stringify(ts6States[ts6States.length - 1]?.chat));
check('ts6: groups derived from serverGroups', (last6?.groups || []).some((g) => g.id === 6), JSON.stringify(last6?.groups));

/* v1.4.1: moves must be announced for EVERYONE — not only own-channel touches.
 * resync() closes the socket (and earlier tests trigger it) — wait until the
 * fake server has a LIVE connection again or the sends vanish silently. */
for (let i = 0; i < 60 && globalThis.ts6Sock?.readyState !== 1; i++) {
  await new Promise((r) => setTimeout(r, 100));
}
await new Promise((r) => setTimeout(r, 350));
globalThis.ts6Sock?.send(JSON.stringify({ type: 'channelCreated', payload: { connectionId: 7, channelId: 77, properties: { name: 'Lab' } } }));
globalThis.ts6Sock?.send(JSON.stringify({ type: 'clientMoved', payload: { connectionId: 7, clientId: 102, oldChannelId: 5, newChannelId: 9, type: 1, visibility: 1 } }));
await new Promise((r) => setTimeout(r, 250));
globalThis.ts6Sock?.send(JSON.stringify({ type: 'clientMoved', payload: { connectionId: 7, clientId: 102, oldChannelId: 9, newChannelId: 77, type: 1, visibility: 1 } }));
await new Promise((r) => setTimeout(r, 250));
{
  const rows = ts6States[ts6States.length - 1]?.chat || [];
  check('ts6: foreign-to-foreign move announced', rows.some((x) => x.kind === 'system' && x.event === 'move' && x.nickname === 'Elsewhere' && x.detail === 'Lab' && x.own === false), JSON.stringify(rows.filter((x) => x.event === 'move')));
  check('ts6: leaving own channel flagged own', rows.some((x) => x.event === 'move' && x.nickname === 'Elsewhere' && x.detail === 'AFK' && x.own === true));
}
globalThis.ts6Sock?.send(JSON.stringify({ type: 'clientMoved', payload: { connectionId: 7, clientId: 102, channelToId: 77, invoker: {} } }));
await new Promise((r) => setTimeout(r, 250));
{
  const rows = ts6States[ts6States.length - 1]?.chat || [];
  check('ts6: duplicate move form deduped', rows.filter((x) => x.event === 'move' && x.nickname === 'Elsewhere' && x.detail === 'Lab').length === 1, JSON.stringify(rows.filter((x) => x.event === 'move')));
}
globalThis.ts6Sock?.send(JSON.stringify({ type: 'clientMoved', payload: { connectionId: 7, clientId: 1, oldChannelId: 5, newChannelId: 9, type: 1, visibility: 1 } }));
await new Promise((r) => setTimeout(r, 250));
{
  const rows = ts6States[ts6States.length - 1]?.chat || [];
  check('ts6: own move announced too', rows.some((x) => x.event === 'move' && x.nickname === 'Me' && x.detail === 'AFK' && x.own === true), JSON.stringify(rows.filter((x) => x.event === 'move')));
}
/* snapshot diff announces moves the event stream missed */
{
  globalThis.ts6Sock?.send(JSON.stringify({ type: 'channelCreated', payload: { connectionId: 7, channelId: 88, properties: { name: 'DiffLab' } } }));
  await new Promise((r) => setTimeout(r, 150));
  const before = (ts6States[ts6States.length - 1]?.chat || []).filter((x) => x.event === 'move' && x.detail === 'DiffLab').length;
  ts6Users.find((u) => u.id === 102).channelId = 88;
  globalThis.ts6Sock?.send(JSON.stringify(ts6AuthDump()));
  await new Promise((r) => setTimeout(r, 500));
  const rows = ts6States[ts6States.length - 1]?.chat || [];
  const after = rows.filter((x) => x.event === 'move' && x.detail === 'DiffLab').length;
  check('ts6: snapshot diff announces missed move', after === before + 1, `before=${before} after=${after}`);
}

/* ---------------- TS3 plugin bridge (Overwolf-style) ---------------- */
const { PluginBridge, PluginState, Ts3WithPluginConnector } = await import('../server/connectors/plugin-bridge.ts');

const pStates = [];
let pStateRef = null;
const pState = new PluginState(() => pStates.push(pStateRef.buildState()));
pStateRef = pState;
const bridge = new PluginBridge(0);
let pluginActive = false;
bridge.on('active', () => { pluginActive = true; });
bridge.on('line', (m) => pState.ingest(m));
bridge.start();
await new Promise((r) => setTimeout(r, 200));
const psock = net.createConnection({ host: '127.0.0.1', port: bridge.getPort() });
await new Promise((r) => psock.on('connect', r));
const sendLine = (obj) => psock.write(JSON.stringify(obj) + '\n');

sendLine({ e: 'hello', v: '1.0.0' });
sendLine({ e: 'snapshot', server: 'Тест', own: 1, owncid: 10, owncname: 'Лобби', clients: [
  { id: 1, name: 'Я', cid: 10, groups: '6', cname: 'Лобби' },
  { id: 2, name: 'Гость', cid: 10, groups: '', cname: 'Лобби' },
  { id: 3, name: 'Дальний', cid: 11, groups: '', cname: 'Другая' },
] });
await new Promise((r) => setTimeout(r, 250));
check('plugin: hello activates the bridge', pluginActive === true);
check('plugin: snapshot fills the channel roster', (pStates.at(-1)?.users || []).length === 2, JSON.stringify(pStates.at(-1)?.users?.map((u) => u.nickname)));
check('plugin: channel name from snapshot', (pStates.at(-1)?.channelName || '') === 'Лобби');

sendLine({ e: 'join', clid: 4, cid: 10, name: 'Новенький', groups: '', cname: 'Лобби' });
sendLine({ e: 'join', clid: 4, cid: 10, name: 'Новенький', groups: '', cname: 'Лобби' });
await new Promise((r) => setTimeout(r, 250));
{
  const rows = pStates.at(-1)?.chat || [];
  check('plugin: join row appears (own channel)', rows.some((m) => m.event === 'join' && m.nickname === 'Новенький' && m.own === true), JSON.stringify(rows.map((m) => m.event)));
  check('plugin: duplicate join deduped', rows.filter((m) => m.event === 'join' && m.nickname === 'Новенький').length === 1);
}

sendLine({ e: 'move', clid: 3, from: 11, to: 10, name: 'Дальний', groups: '', cname: 'Лобби' });
sendLine({ e: 'chat', kind: 'server', clid: 2, name: 'Гость', text: 'Welcome' });
sendLine({ e: 'talk', clid: 2, s: 1 });
sendLine({ e: 'client', clid: 2, name: 'Гость', groups: '6,12' });
sendLine({ e: 'poke', clid: 3, name: 'Дальний', text: 'привет' });
await new Promise((r) => setTimeout(r, 250));
{
  const rows = pStates.at(-1)?.chat || [];
  const users = pStates.at(-1)?.users || [];
  check('plugin: global move row with own flag', rows.some((m) => m.event === 'move' && m.nickname === 'Дальний' && m.own === true && m.detail === 'Лобби'), JSON.stringify(rows.filter((m) => m.event === 'move')));
  check('plugin: server chat row verbatim', rows.some((m) => m.kind === 'server' && m.text === 'Welcome'));
  check('plugin: poke row', rows.some((m) => m.kind === 'poke' && m.text === 'привет'));
  check('plugin: group-change row', rows.some((m) => m.event === 'group' && m.userId === '2'));
  check('plugin: talk state updates', users.some((u) => u.id === '2' && u.talking === true));
}

// The plugin must take over from the ClientQuery fallback while it is alive.
// Short idle window + quick fallback keep the swap inside the test budget.
const autoConn = new Ts3WithPluginConnector('127.0.0.1', 1, '', 0, 300, 100);
const autoStates = [];
autoConn.on('state', (st) => autoStates.push(st));
autoConn.start();
await new Promise((r) => setTimeout(r, 150));
check('plugin: lazy fallback — query poller armed before the plugin', autoConn.getQuery() !== null);
const psock2 = net.createConnection({ host: '127.0.0.1', port: autoConn.getBridge().getPort() });
await new Promise((r) => psock2.on('connect', r));
psock2.write(JSON.stringify({ e: 'hello', v: '1.0.0' }) + '\n');
psock2.write(JSON.stringify({ e: 'snapshot', server: 'Тест', own: 1, owncid: 5, clients: [{ id: 1, name: 'Я', cid: 5, groups: '' }] }) + '\n');
await new Promise((r) => setTimeout(r, 350));
check('plugin: plugin takes over from ClientQuery', autoStates.some((st) => st.detail === 'TS3 plugin' && st.users.length === 1), JSON.stringify(autoStates.map((st) => st.detail)));
check('plugin: query poller stops while the plugin is alive', autoConn.getQuery() === null);

// Swap-hold: when the plugin goes away the last plugin picture must survive
// until ClientQuery boots — no blank overlay during the swap.
{
  const held = autoStates.filter((st) => st.detail === 'TS3 plugin' && st.users.length === 1).length;
  psock2.destroy();
  await new Promise((r) => setTimeout(r, 900));
  const after = autoStates.slice(-6);
  check('plugin: state held through the fallback swap', held > 0 && after.some((st) => st.users.length === 1), JSON.stringify(after.map((st) => `${st.status}:${st.users.length}`)));
  await new Promise((r) => setTimeout(r, 400));
  check('plugin: fallback query re-armed after the plugin died', autoConn.getQuery() !== null);
}

// v1.4.6 «one at a time»: when the plugin connects first, the ClientQuery
// poller must NEVER start.
{
  const lazyConn = new Ts3WithPluginConnector('127.0.0.1', 1, '', 0, 5000, 3000);
  const lazyStates = [];
  lazyConn.on('state', (st) => lazyStates.push(st));
  lazyConn.start();
  await new Promise((r) => setTimeout(r, 150));
  const lsock = net.createConnection({ host: '127.0.0.1', port: lazyConn.getBridge().getPort() });
  await new Promise((r) => lsock.on('connect', r));
  lsock.write(JSON.stringify({ e: 'hello', v: '1.0.0' }) + '\n');
  lsock.write(JSON.stringify({ e: 'snapshot', server: 'Тест', own: 1, owncid: 5, clients: [{ id: 1, name: 'Я', cid: 5, groups: '' }] }) + '\n');
  await new Promise((r) => setTimeout(r, 700));
  check(
    'plugin: query poller never starts when the plugin connects first',
    lazyConn.getQuery() === null && lazyStates.some((st) => st.detail === 'TS3 plugin'),
    JSON.stringify(lazyStates.map((st) => st.detail)),
  );
  lsock.destroy();
  lazyConn.stop();
}

// v1.4.7: connecting must NOT flood the feed with "everyone joined".
{
  const fl = new PluginState(() => {});
  const rowsOf = () => fl.buildState().chat.filter((m) => m.kind === 'system');
  // 1) empty baseline → populated (server connect) stays silent
  fl.ingest({ e: 'snapshot', server: 'S', own: 0, owncid: 0, owncname: '', clients: [] });
  fl.ingest({ e: 'snapshot', server: 'S', own: 5, owncid: 10, owncname: 'A', clients: [
    { id: 5, name: 'me', cid: 10, groups: '1', cname: 'A' },
    { id: 6, name: 'a', cid: 10, groups: '2', cname: 'A' },
    { id: 7, name: 'b', cid: 10, groups: '3', cname: 'A' },
  ] });
  check('flood: connect baseline is silent', rowsOf().length === 0, String(rowsOf().length));
  // 2) one real join between populated snapshots is announced
  fl.ingest({ e: 'snapshot', server: 'S', own: 5, owncid: 10, owncname: 'A', clients: [
    { id: 5, name: 'me', cid: 10, groups: '1', cname: 'A' },
    { id: 6, name: 'a', cid: 10, groups: '2', cname: 'A' },
    { id: 7, name: 'b', cid: 10, groups: '3', cname: 'A' },
    { id: 8, name: 'c', cid: 10, groups: '4', cname: 'A' },
  ] });
  const one = rowsOf();
  check('flood: a single join is still announced', one.length === 1 && one[0].event === 'join' && one[0].nickname === 'c', JSON.stringify(one));
  // 3) a mass resync (everyone replaced) stays silent
  fl.ingest({ e: 'snapshot', server: 'S', own: 9, owncid: 10, owncname: 'A', clients: [
    { id: 9, name: 'me2', cid: 10, groups: '1', cname: 'A' },
    { id: 10, name: 'x', cid: 10, groups: '2', cname: 'A' },
    { id: 11, name: 'y', cid: 10, groups: '3', cname: 'A' },
    { id: 12, name: 'z', cid: 10, groups: '4', cname: 'A' },
    { id: 13, name: 'w', cid: 10, groups: '5', cname: 'A' },
    { id: 14, name: 'v', cid: 10, groups: '6', cname: 'A' },
  ] });
  check('flood: mass resync is silent', rowsOf().length === 1, String(rowsOf().length));
}

// v1.4.4: an own move must switch the channel window (blank-overlay bug).
{
  const mv = new PluginState(() => {});
  mv.ingest({ e: 'snapshot', server: 'S', own: 5, owncid: 10, owncname: 'A', clients: [
    { id: 5, name: 'me', cid: 10, groups: '1', cname: 'A' },
    { id: 6, name: 'you', cid: 10, groups: '2', cname: 'A' },
    { id: 7, name: 'other', cid: 20, groups: '3', cname: 'B' },
  ] });
  let st = mv.buildState();
  check('plugin state: snapshot roster is the own channel', st.channelName === 'A' && st.users.length === 2, `${st.channelName}:${st.users.length}`);
  mv.ingest({ e: 'move', clid: 5, from: 10, to: 20, name: 'me', groups: '1', cname: 'B' });
  st = mv.buildState();
  check(
    'plugin state: own move switches channel roster + name',
    st.channelName === 'B' && st.channelId === '20' && st.users.length === 2 && st.users.some((u) => u.id === '7'),
    `${st.channelName}:${st.channelId}:${st.users.map((u) => u.id).join(',')}`,
  );
  mv.ingest({ e: 'move', clid: 6, from: 10, to: 20, name: 'you', groups: '2', cname: 'B' });
  st = mv.buildState();
  check('plugin state: moves into the new channel land in the roster', st.users.length === 3, st.users.map((u) => u.id).join(','));
  mv.ingest({ e: 'disconnect' });
  mv.ingest({ e: 'snapshot', server: 'S', own: 5, owncid: 30, owncname: 'C', clients: [{ id: 5, name: 'me', cid: 30, groups: '1', cname: 'C' }] });
  st = mv.buildState();
  check('plugin state: fresh snapshot after reconnect re-anchors the channel', st.channelName === 'C' && st.users.length === 1, `${st.channelName}:${st.users.length}`);
}

// Heartbeat semantics: pings keep the plugin counted as alive; a silent
// socket eventually marks it idle (short window via constructor).
{
  const hBridge = new PluginBridge(0, 400);
  let hActive = 0;
  let hIdle = 0;
  hBridge.on('active', () => { hActive += 1; });
  hBridge.on('idle', () => { hIdle += 1; });
  hBridge.start();
  await new Promise((r) => setTimeout(r, 150));
  const hs = net.createConnection({ host: '127.0.0.1', port: hBridge.getPort() });
  await new Promise((r) => hs.on('connect', r));
  hs.write(JSON.stringify({ e: 'hello', v: '1.1.0' }) + '\n');
  await new Promise((r) => setTimeout(r, 150));
  check('bridge: hello marks the plugin active', hActive === 1 && hBridge.active() === true);
  for (let i = 0; i < 4; i++) {
    hs.write('{"e":"ping"}\n');
    await new Promise((r) => setTimeout(r, 150));
  }
  check('bridge: heartbeats keep the plugin active', hIdle === 0 && hBridge.active() === true);
  hs.destroy();
  await new Promise((r) => setTimeout(r, 1000));
  check('bridge: dead plugin marked idle after the quiet window', hIdle === 1 && hBridge.active() === false);
  hBridge.stop();
}

psock.destroy();
psock2.destroy();
bridge.stop();
autoConn.stop();

/* ---- teardown ---- */
c3.stop();
c6.stop();
ts3Server.close();
ts6Wss.close();

const failed = results.filter((r) => !r.ok);
console.log(`\n${results.length - failed.length}/${results.length} checks passed`);
process.exit(failed.length ? 1 : 0);
