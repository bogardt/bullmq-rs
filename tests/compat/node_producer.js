/**
 * Adds a job using BullMQ Node.js for bullmq-rs to process.
 *
 * After running this, start a Rust worker to process the job:
 *   cargo run --example basic_worker
 */
const { Queue } = require('bullmq');

const REDIS_URL = process.env.REDIS_URL || 'redis://127.0.0.1:6379';

async function main() {
  const queue = new Queue('emails', {
    connection: { url: REDIS_URL },
  });

  try {
    const job = await queue.add('welcome', {
      to: 'user@example.com',
      subject: 'Hello from Node.js',
      body: 'This job was created by BullMQ Node.js',
    });

    console.log(`Added job ${job.id} via BullMQ Node.js`);
    console.log('Now run: cargo run --example basic_worker');
  } finally {
    await queue.close();
  }
}

main().catch((err) => {
  console.error('FAILED:', err.message);
  process.exit(1);
});
