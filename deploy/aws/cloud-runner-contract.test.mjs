import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import test from 'node:test'

const template = readFileSync(new URL('./cloud-runner.yaml', import.meta.url), 'utf8')
const applyScript = readFileSync(new URL('./apply-cloud-runner.sh', import.meta.url), 'utf8')
const infrastructureWorkflow = readFileSync(
  new URL('../../.github/workflows/scope-aws-infrastructure.yml', import.meta.url),
  'utf8',
)

function between(source, start, end) {
  const startIndex = source.indexOf(start)
  const endIndex = source.indexOf(end, startIndex + start.length)
  assert.notEqual(startIndex, -1, `missing ${start}`)
  assert.notEqual(endIndex, -1, `missing ${end}`)
  return source.slice(startIndex, endIndex)
}

test('private registry credentials remain an optional exact secret grant', () => {
  const parameters = between(template, 'Parameters:', 'Conditions:')
  assert.match(parameters, /RegistryCredentialsSecretArn:\n\s+Type: String\n\s+Default: ""/)
  assert.match(
    parameters,
    /AllowedPattern: "\^\$\|\^arn:\(aws\|aws-us-gov\|aws-cn\):secretsmanager:/,
  )
  assert.match(
    template,
    /HasRegistryCredentialsSecret: !Not \[!Equals \[!Ref RegistryCredentialsSecretArn, ""\]\]/,
  )

  const executionRole = between(template, '  RunnerTaskExecutionRole:', '  RailwayDispatcherUser:')
  assert.match(
    executionRole,
    /- !If\n\s+- HasRegistryCredentialsSecret\n\s+- Sid: ReadPrivateRegistryCredentials[\s\S]*?Action: secretsmanager:GetSecretValue\n\s+Resource: !Ref RegistryCredentialsSecretArn\n\s+- !Ref AWS::NoValue/,
  )
  assert.doesNotMatch(
    executionRole,
    /Action: secretsmanager:GetSecretValue\n\s+Resource: ["']?\*["']?/,
  )
})

test('the Railway dispatcher is denied direct secret values and discovery', () => {
  const dispatcher = between(template, '  RailwayDispatcherPolicy:', '  RunnerBudget:')
  const deny = between(dispatcher, '          - Sid: DenySecretReads', '          - Sid: ListTaskDefinitions')
  assert.match(deny, /Effect: Deny/)
  for (const action of [
    'BatchGetSecretValue',
    'DescribeSecret',
    'GetSecretValue',
    'ListSecretVersionIds',
    'ListSecrets',
  ]) {
    assert.match(deny, new RegExp(`secretsmanager:${action}`))
  }
  assert.match(deny, /Resource: "\*"/)
  assert.doesNotMatch(
    dispatcher.replace(deny, ''),
    /Effect: Allow[\s\S]{0,240}secretsmanager:(?:BatchGet|Describe|Get|List)/,
  )
})

test('the same optional ARN reaches CloudFormation without becoming a secret value', () => {
  assert.match(
    applyScript,
    /registry_credentials_secret_arn="\$\{SCOPE_REGISTRY_CREDENTIALS_SECRET_ARN:-\}"/,
  )
  assert.match(
    applyScript,
    /ParameterKey=RegistryCredentialsSecretArn,ParameterValue=\$registry_credentials_secret_arn/,
  )
  assert.match(
    infrastructureWorkflow,
    /SCOPE_REGISTRY_CREDENTIALS_SECRET_ARN: \$\{\{ vars\.SCOPE_REGISTRY_CREDENTIALS_SECRET_ARN \}\}/,
  )
  assert.doesNotMatch(infrastructureWorkflow, /secrets\.SCOPE_REGISTRY_CREDENTIALS_SECRET_ARN/)
})

test('managed instances stay opt-in and retire idle experiment hosts immediately', () => {
  const parameters = between(template, 'Parameters:', 'Conditions:')
  assert.match(
    parameters,
    /RunnerCapacity:\n\s+Type: String\n\s+Default: FARGATE[\s\S]*?- MANAGED_INSTANCES/,
  )
  const provider = between(template, '  ManagedCapacityProvider:', '  RunnerClusterCapacityProviders:')
  assert.match(provider, /Condition: UseManagedInstances/)
  assert.match(provider, /ScaleInAfter: 0/)
  assert.match(provider, /CapacityOptionType: ON_DEMAND/)
  assert.match(provider, /AllowedInstanceTypes:\n\s+- !Ref ManagedInstanceType/)
  assert.match(provider, /VCpuCount:\n\s+Min: 4\n\s+Max: 4/)
  assert.match(provider, /MemoryMiB:\n\s+Min: 16384\n\s+Max: 16384/)
})

test('task-family cleanup reflects the AWS authorization boundary', () => {
  const parameters = between(template, 'Parameters:', 'Conditions:')
  assert.match(
    parameters,
    /TaskFamilyPrefix:\n\s+Type: String\n\s+Default: scope-runner-/,
  )
  const dispatcher = between(template, '  RailwayDispatcherPolicy:', '  RunnerBudget:')
  const deregister = between(
    dispatcher,
    '          - Sid: DeregisterAttemptTaskDefinitions',
    '          - Sid: RunScopeTaskDefinitions',
  )
  assert.match(deregister, /Resource: "\*"/)
  assert.match(
    deregister,
    /ECS task-definition lifecycle actions do not support resource-level\n\s+# permissions/,
  )
  assert.match(
    applyScript,
    /ParameterKey=TaskFamilyPrefix,ParameterValue=\$task_family_prefix/,
  )
})
