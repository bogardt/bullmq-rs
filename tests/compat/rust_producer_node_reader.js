/**
 * Verifies that jobs added by bullmq-rs can be read by BullMQ Node.js.
 *
 * Run `cargo run --example basic_queue` first to populate the queue,
 * then run this script.
 */
const { Queue } = require('bullmq');

const REDIS_URL = process.env.REDIS_URL || 'redis://127.0.0.1:6379';

async function main() {
  const queue = new Queue('emails', {
    connection: { url: REDIS_URL },
  });

  try {
    const counts = await queue.getJobCounts();
    console.log('Job counts:', counts);

    if (counts.waiting === 0 && counts.delayed === 0) {
      console.log('No jobs found. Run `cargo run --example basic_queue` first.');
      process.exit(1);
    }

    // Try to read a waiting job
    const jobs = await queue.getJobs(['waiting', 'delayed'], 0, 10);
    console.log(`Found ${jobs.length} jobs`);

    for (const job of jobs) {
      console.log(`  Job ${job.id}: name=${job.name}, data=${JSON.stringify(job.data)}`);
      // Verify we can read the data without WRONGTYPE errors
      if (!job.data) {
        console.error(`  ERROR: Job ${job.id} has no data`);
        process.exit(1);
      }
    }

    console.log('SUCCESS: All jobs readable by BullMQ Node.js');
  } finally {
    await queue.close();
  }
}

main().catch((err) => {
  console.error('FAILED:', err.message);
  process.exit(1);
});
