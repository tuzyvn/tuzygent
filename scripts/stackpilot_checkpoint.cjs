#!/usr/bin/env node
'use strict';

const BASE_URL = (process.env.STACKPILOT_URL || 'http://127.0.0.1:49382').replace(/\/+$/, '');
const TOKEN = process.env.STACKPILOT_TOKEN || '';

function usage() {
  console.error([
    'Usage (requires running StackPilot service at STACKPILOT_URL, default http://127.0.0.1:49382):',
    '  node scripts/stackpilot_checkpoint.cjs list <envIdOrName>',
    '  node scripts/stackpilot_checkpoint.cjs create <envIdOrName> <serviceId> [snapshotName]',
    '  node scripts/stackpilot_checkpoint.cjs restore <envIdOrName> <fileName>',
    '  node scripts/stackpilot_checkpoint.cjs delete <envIdOrName> <fileName>'
  ].join('\n'));
  process.exit(2);
}

async function api(method, pathname, body) {
  const headers = { 'Content-Type': 'application/json' };
  if (TOKEN) headers.Authorization = `Bearer ${TOKEN}`;

  let res;
  try {
    res = await fetch(`${BASE_URL}${pathname}`, {
      method,
      headers,
      body: body === undefined ? undefined : JSON.stringify(body)
    });
  } catch (err) {
    throw new Error(`Cannot reach StackPilot at ${BASE_URL}: ${err.message}`);
  }

  const text = await res.text();
  let data;
  try {
    data = text ? JSON.parse(text) : {};
  } catch {
    data = { raw: text };
  }

  if (!res.ok) {
    throw new Error(data.error || data.message || `HTTP ${res.status} from ${pathname}`);
  }
  return data;
}

async function resolveEnvironmentId(envQuery) {
  const { environments = [] } = await api('GET', '/api/environments');
  const found = environments.find(
    e => e.id === envQuery || (e.name && e.name.toLowerCase() === envQuery.toLowerCase())
  );
  if (!found) {
    throw new Error(`StackPilot environment not found: ${envQuery}`);
  }
  return found.id;
}

async function main() {
  const [, , cmd, envQuery, arg3, arg4] = process.argv;
  if (!cmd || !envQuery) usage();

  const envId = await resolveEnvironmentId(envQuery);

  if (cmd === 'list') {
    const data = await api('GET', `/api/environments/${envId}/snapshots`);
    console.log(JSON.stringify({ environmentId: envId, ...data }, null, 2));
    return;
  }

  if (cmd === 'create') {
    if (!arg3) usage();
    const data = await api('POST', `/api/environments/${envId}/snapshots`, {
      serviceId: arg3,
      name: arg4 || 'tuzygent-checkpoint'
    });
    console.log(JSON.stringify({ environmentId: envId, ...data }, null, 2));
    return;
  }

  if (cmd === 'restore') {
    if (!arg3) usage();
    const data = await api('POST', `/api/environments/${envId}/snapshots/restore`, {
      fileName: arg3
    });
    console.log(JSON.stringify({ environmentId: envId, ...data }, null, 2));
    return;
  }

  if (cmd === 'delete') {
    if (!arg3) usage();
    const data = await api('DELETE', `/api/environments/${envId}/snapshots`, {
      fileName: arg3
    });
    console.log(JSON.stringify({ environmentId: envId, ...data }, null, 2));
    return;
  }

  usage();
}

main().catch(err => {
  console.error(JSON.stringify({ error: err.message }));
  process.exit(1);
});
