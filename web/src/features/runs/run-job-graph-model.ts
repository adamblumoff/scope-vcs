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
  key: string
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
const EDGE_LANE_GAP = 10
const EDGE_LANE_PADDING = 12
const MAX_LONG_EDGE_LANES = 12

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
  const dependencies = keysByLayer.flatMap((keys, layer) =>
    keys.flatMap((to) => (byKey.get(to)?.job.needs ?? []).map((from) => ({
      endLayer: layer,
      from,
      key: edgeKey(from, to),
      startLayer: layers.get(from) ?? 0,
      to,
    }))))
  const longEdgeLanes = assignLongEdgeLanes(
    dependencies.filter((edge) => edge.endLayer - edge.startLayer > 1),
  )
  const edgeLaneCount = longEdgeLanes.size > 0
    ? Math.max(...longEdgeLanes.values()) + 1
    : 0
  const edgeCorridorHeight = edgeLaneCount > 0
    ? EDGE_LANE_PADDING + edgeLaneCount * EDGE_LANE_GAP
    : 0
  const largestLayer = Math.max(1, ...keysByLayer.map((keys) => keys.length))
  const contentHeight = largestLayer * JOB_GRAPH_NODE_HEIGHT +
    (largestLayer - 1) * ROW_GAP
  const height = contentHeight + edgeCorridorHeight + GRAPH_PADDING * 2
  const width = Math.max(
    JOB_GRAPH_NODE_WIDTH + GRAPH_PADDING * 2,
    layerCount * JOB_GRAPH_NODE_WIDTH +
      Math.max(0, layerCount - 1) * LAYER_GAP +
      GRAPH_PADDING * 2,
  )
  const nodes = keysByLayer.flatMap((keys, layer) => {
    const layerHeight = keys.length * JOB_GRAPH_NODE_HEIGHT +
      Math.max(0, keys.length - 1) * ROW_GAP
    const top = GRAPH_PADDING + edgeCorridorHeight +
      (contentHeight - layerHeight) / 2
    return keys.map((key, row) => ({
      key,
      layer,
      needs: byKey.get(key)?.job.needs ?? [],
      x: GRAPH_PADDING + layer * (JOB_GRAPH_NODE_WIDTH + LAYER_GAP),
      y: top + row * (JOB_GRAPH_NODE_HEIGHT + ROW_GAP),
    }))
  })
  const positions = new Map(nodes.map((node) => [node.key, node]))
  const edges = dependencies.map((edge) => {
    const source = positions.get(edge.from)
    const target = positions.get(edge.to)
    if (!source) throw new Error(`Run graph dependency ${edge.from} is missing`)
    if (!target) throw new Error(`Run graph job ${edge.to} is missing`)
    const startX = source.x + JOB_GRAPH_NODE_WIDTH
    const startY = source.y + JOB_GRAPH_NODE_HEIGHT / 2
    const endX = target.x
    const endY = target.y + JOB_GRAPH_NODE_HEIGHT / 2
    const lane = longEdgeLanes.get(edge.key)
    return {
      from: edge.from,
      key: edge.key,
      path: lane === undefined
        ? adjacentEdgePath(startX, startY, endX, endY)
        : longEdgePath(
            startX,
            startY,
            endX,
            endY,
            GRAPH_PADDING + EDGE_LANE_PADDING / 2 + lane * EDGE_LANE_GAP,
          ),
      to: edge.to,
    }
  })

  return { edges, height, nodes, width }
}

function assignLongEdgeLanes(
  edges: Array<{
    endLayer: number
    key: string
    startLayer: number
  }>,
) {
  const lanes = new Map<string, number>()
  const occupiedThrough: number[] = []
  const ordered = [...edges].sort((left, right) =>
    left.startLayer - right.startLayer ||
    left.endLayer - right.endLayer ||
    left.key.localeCompare(right.key))
  for (const edge of ordered) {
    let lane = occupiedThrough.findIndex((endLayer) => endLayer <= edge.startLayer)
    if (lane < 0 && occupiedThrough.length < MAX_LONG_EDGE_LANES) {
      lane = occupiedThrough.length
    }
    if (lane < 0) lane = stableLane(edge.key)
    occupiedThrough[lane] = Math.max(occupiedThrough[lane] ?? 0, edge.endLayer)
    lanes.set(edge.key, lane)
  }
  return lanes
}

function adjacentEdgePath(
  startX: number,
  startY: number,
  endX: number,
  endY: number,
) {
  const midpoint = startX + (endX - startX) / 2
  return `M ${startX} ${startY} C ${midpoint} ${startY}, ${midpoint} ${endY}, ${endX} ${endY}`
}

function longEdgePath(
  startX: number,
  startY: number,
  endX: number,
  endY: number,
  laneY: number,
) {
  return `M ${startX} ${startY} C ${startX + 12} ${startY}, ${startX + 16} ${laneY}, ${startX + 24} ${laneY} L ${endX - 24} ${laneY} C ${endX - 16} ${laneY}, ${endX - 12} ${endY}, ${endX} ${endY}`
}

function edgeKey(from: string, to: string) {
  return JSON.stringify([from, to])
}

function stableLane(key: string) {
  let hash = 0
  for (const character of key) hash = (hash * 31 + character.charCodeAt(0)) >>> 0
  return hash % MAX_LONG_EDGE_LANES
}
