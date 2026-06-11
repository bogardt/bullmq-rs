/**
 * Adds a job using BullMQ Node.js for bullmq-rs to process.
 *
 * After running this, process the job with:
 *   cargo run --example compat_worker
 */
const { Queue } = require('bullmq');

const REDIS_URL = process.env.REDIS_URL || 'redis://127.0.0.1:6379';
const QUEUE_NAME = process.env.COMPAT_N2R_QUEUE || 'compat-node-to-rust';

async function main() {
  const queue = new Queue(QUEUE_NAME, {
    connection: { url: REDIS_URL },
  });

  try {
    const job = await queue.add('welcome', {
      to: 'user@example.com',
      subject: 'Hello from Node.js',
      body: 'This job was created by BullMQ Node.js',
    });

    console.log(`Added job ${job.id} via BullMQ Node.js on queue '${QUEUE_NAME}'`);
    console.log('Now run: cargo run --example compat_worker');
  } finally {
    await queue.close();
  }
}

main().catch((err) => {
  console.error('FAILED:', err.message);
  process.exit(1);
});
