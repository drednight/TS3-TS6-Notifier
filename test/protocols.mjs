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
check('ts6: groups derived from serverGroups', (last6?.groups || []).some((g) => g.id === 6), JSON.stringify(last6?.groups));

/* ---- teardown ---- */
c3.stop();
c6.stop();
ts3Server.close();
ts6Wss.close();

const failed = results.filter((r) => !r.ok);
console.log(`\n${results.length - failed.length}/${results.length} checks passed`);
process.exit(failed.length ? 1 : 0);
