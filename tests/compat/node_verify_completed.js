/**
 * Verifies from BullMQ Node.js that the job produced by node_producer.js
 * was completed by the bullmq-rs worker (compat_worker example).
 */
const { Queue } = require('bullmq');

const REDIS_URL = process.env.REDIS_URL || 'redis://127.0.0.1:6379';
const QUEUE_NAME = process.env.COMPAT_N2R_QUEUE || 'compat-node-to-rust';

function fail(message) {
  console.error(`FAILED: ${message}`);
  process.exit(1);
}

async function main() {
  const queue = new Queue(QUEUE_NAME, {
    connection: { url: REDIS_URL },
  });

  try {
    const counts = await queue.getJobCounts();
    console.log('Job counts:', counts);

    if (counts.failed > 0) fail(`${counts.failed} job(s) failed`);
    if (counts.completed < 1) fail('expected at least 1 completed job');

    const jobs = await queue.getJobs(['completed'], 0, 10);
    for (const job of jobs) {
      console.log(`  Completed job ${job.id}: name=${job.name}, finishedOn=${job.finishedOn}`);
      if (!job.finishedOn) fail(`completed job ${job.id} has no finishedOn timestamp`);
    }

    console.log('SUCCESS: Rust-completed job state readable by BullMQ Node.js');
  } finally {
    await queue.close();
  }
}

main().catch((err) => {
  console.error('FAILED:', err.message);
  process.exit(1);
});
