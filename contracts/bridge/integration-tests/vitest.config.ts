import { defineConfig } from 'vitest/config';

export default defineConfig({
  test: {
    globals: true,
    environment: 'node',
    testTimeout: 120000,
    hookTimeout: 300000,
    pool: 'forks',
    poolOptions: {
      forks: {
        singleFork: true,
        isolate: false,
      },
    },
    include: ['src/**/*.{test,spec}.{js,mjs,cjs,ts,mts,cts}'],
    exclude: ['node_modules', 'dist'],
    reporters: ['verbose'],
    retry: 0,
    sequence: {
      shuffle: false,
    },
  },
});
