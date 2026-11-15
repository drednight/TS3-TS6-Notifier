import { join } from 'node:path';
import { startServer } from './app.js';

const ROOT = join(import.meta.dirname, '..');
const PORT = Number(process.env['PORT'] || 3000);
const HOST = process.env['HOST'] || '0.0.0.0';

startServer({
  port: PORT,
  host: HOST,
  dataDir: join(ROOT, 'data'),
  publicDir: join(ROOT, 'public'),
}).catch((err) => {
  console.error('[server] failed to start:', err);
  process.exit(1);
});
