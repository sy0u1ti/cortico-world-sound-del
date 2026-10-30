import { HelperClient } from '../../dist/helper-client.js';

const helper = new HelperClient({
  dataDir: process.argv[2],
  maxMuteSeconds: 10,
  desired: () => true,
  changed() {},
  extraArgs: ['--simulation-file', process.argv[3]],
  expectedBackend: 'simulation',
});
await helper.start();
