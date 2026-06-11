/**
 * Verifies that jobs added by bullmq-rs can be read by BullMQ Node.js.
 *
 * Run `cargo run --example compat_producer` first to populate the queue,
 * then run this script.
 */
const { Queue } = require('bullmq');

const REDIS_URL = process.env.REDIS_URL || 'redis://127.0.0.1:6379';
const QUEUE_NAME = process.env.COMPAT_R2N_QUEUE || 'compat-rust-to-node';

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

    if (counts.waiting < 1) fail('expected at least 1 waiting job');
    if (counts.delayed < 1) fail('expected at least 1 delayed job');
    if (counts.prioritized < 1) fail('expected at least 1 prioritized job');

    const jobs = await queue.getJobs(['waiting', 'delayed', 'prioritized'], 0, 10);
    console.log(`Found ${jobs.length} jobs`);

    const byName = {};
    for (const job of jobs) {
      console.log(`  Job ${job.id}: name=${job.name}, data=${JSON.stringify(job.data)}, opts=${JSON.stringify(job.opts)}`);
      if (!job.data || !job.data.to || !job.data.subject || !job.data.body) {
        fail(`job ${job.id} data not readable or missing fields`);
      }
      byName[job.name] = job;
    }

    if (!byName.welcome) fail("missing plain job 'welcome'");
    if (!byName.reminder) fail("missing delayed job 'reminder'");
    if (!byName.urgent) fail("missing prioritized job 'urgent'");

    if (!byName.reminder.opts || !byName.reminder.opts.delay) {
      fail("delayed job 'reminder' has no delay in opts");
    }
    if (!byName.urgent.opts || byName.urgent.opts.priority !== 5) {
      fail(`prioritized job 'urgent' has priority ${byName.urgent.opts && byName.urgent.opts.priority}, expected 5`);
    }

    console.log('SUCCESS: all bullmq-rs jobs readable by BullMQ Node.js');
  } finally {
    await queue.close();
  }
}

main().catch((err) => {
  console.error('FAILED:', err.message);
  process.exit(1);
});
