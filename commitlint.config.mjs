export default {
  extends: ['@commitlint/config-conventional'],
  rules: {
    // Dependabot cargo PRs use the "deps" prefix (see .github/dependabot.yml).
    'type-enum': [
      2,
      'always',
      [
        'build',
        'chore',
        'ci',
        'deps',
        'docs',
        'feat',
        'fix',
        'perf',
        'refactor',
        'release',
        'revert',
        'style',
        'test',
      ],
    ],
    // Dependabot bodies contain long release-note tables and URLs.
    'body-max-line-length': [0, 'always', Infinity],
    'footer-max-line-length': [0, 'always', Infinity],
  },
};
