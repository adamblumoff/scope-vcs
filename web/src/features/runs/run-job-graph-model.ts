type JobGraphInput = {
  job: {
    key: string
    needs: readonly string[]
  }
}

export type RunJobGraphNode = {
  key: string
  layer: number
  needs: readonly string[]
  x: number
  y: number
}

export type RunJobGraphEdge = {
  from: string
  path: string
  to: string
}

export type RunJobGraphLayout = {
  edges: RunJobGraphEdge[]
  height: number
  nodes: RunJobGraphNode[]
  width: number
}

export const JOB_GRAPH_NODE_WIDTH = 208
export const JOB_GRAPH_NODE_HEIGHT = 84
const LAYER_GAP = 88
const ROW_GAP = 20
const GRAPH_PADDING = 16

export function buildRunJobGraph(
  jobs: readonly JobGraphInput[],
): RunJobGraphLayout {
  const byKey = new Map(jobs.map((job) => [job.job.key, job]))
  const layers = new Map<string, number>()
  const visiting = new Set<string>()

  function layerFor(key: string): number {
    const existing = layers.get(key)
    if (existing !== undefined) return existing
    const current = byKey.get(key)
    if (!current) throw new Error(`Run graph dependency ${key} is missing`)
    if (visiting.has(key)) throw new Error('Run graph contains a dependency cycle')
    visiting.add(key)
    const layer = current.job.needs.reduce(
      (maximum, dependency) => Math.max(maximum, layerFor(dependency) + 1),
      0,
    )
    visiting.delete(key)
    layers.set(key, layer)
    return layer
  }

  for (const job of jobs) layerFor(job.job.key)
  const layerCount = Math.max(0, ...layers.values()) + 1
  const keysByLayer = Array.from({ length: layerCount }, () => [] as string[])
  for (const [key, layer] of layers) keysByLayer[layer]?.push(key)
  for (const keys of keysByLayer) keys.sort()
  const largestLayer = Math.max(1, ...keysByLayer.map((keys) => keys.length))
  const contentHeight = largestLayer * JOB_GRAPH_NODE_HEIGHT +
    (largestLayer - 1) * ROW_GAP
  const height = contentHeight + GRAPH_PADDING * 2
  const width = Math.max(
    JOB_GRAPH_NODE_WIDTH + GRAPH_PADDING * 2,
    layerCount * JOB_GRAPH_NODE_WIDTH +
      Math.max(0, layerCount - 1) * LAYER_GAP +
      GRAPH_PADDING * 2,
  )
  const nodes = keysByLayer.flatMap((keys, layer) => {
    const layerHeight = keys.length * JOB_GRAPH_NODE_HEIGHT +
      Math.max(0, keys.length - 1) * ROW_GAP
    const top = GRAPH_PADDING + (contentHeight - layerHeight) / 2
    return keys.map((key, row) => ({
      key,
      layer,
      needs: byKey.get(key)?.job.needs ?? [],
      x: GRAPH_PADDING + layer * (JOB_GRAPH_NODE_WIDTH + LAYER_GAP),
      y: top + row * (JOB_GRAPH_NODE_HEIGHT + ROW_GAP),
    }))
  })
  const positions = new Map(nodes.map((node) => [node.key, node]))
  const edges = nodes.flatMap((node) => node.needs.map((dependency) => {
    const source = positions.get(dependency)
    if (!source) throw new Error(`Run graph dependency ${dependency} is missing`)
    const startX = source.x + JOB_GRAPH_NODE_WIDTH
    const startY = source.y + JOB_GRAPH_NODE_HEIGHT / 2
    const endX = node.x
    const endY = node.y + JOB_GRAPH_NODE_HEIGHT / 2
    const midpoint = startX + (endX - startX) / 2
    return {
      from: dependency,
      path: `M ${startX} ${startY} C ${midpoint} ${startY}, ${midpoint} ${endY}, ${endX} ${endY}`,
      to: node.key,
    }
  }))

  return { edges, height, nodes, width }
}
