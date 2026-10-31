import type { ColumnDef } from '@tanstack/react-table'
import { formatShortDate, type AuditEvent } from '../lib/api'
import { useAuditLog } from '../lib/queries'
import { DataTable, Empty, ErrorBox, Loading, PageHeader } from '../components/common'
import { CLOUD } from '../lib/edition'

const ACTION_LABELS: Record<string, string> = {
  'account.name_updated': '修改帐号名称',
  'account.settings_updated': '修改帐号设置',
  'channel.connection_previewed': '测试渠道连接',
  'channel.created': '创建渠道',
  'channel.deleted': '删除渠道',
  'channel.models_previewed': '预览渠道模型',
  'channel.models_synced': '同步渠道模型',
  'channel.models_updated': '更新渠道模型',
  'channel.monitor_run': '执行渠道探测',
  'channel.monitor_updated': '修改渠道监控配置',
  'channel.updated': '修改渠道',
  'quality_monitor.created': '添加模型监控',
  'quality_monitor.updated': '编辑模型监控',
  'quality_monitor.run': '执行服务体检',
  'quality_monitor.deleted': '删除模型监控',
  'quality_monitor.deep_check': '发起真伪检测',
  'quality_monitor.authenticity_acknowledged': '确认真伪检测风险提示',
  'platform.probe_template_created': '添加探针模板',
  'platform.probe_template_retired': '停用探针模板',
  'platform.probe_templates_defaults_imported': '导入默认探针模板',
  'platform.probe_pool_published': '发布提示词池',
  'platform.baseline_target_updated': '修改基准模型',
  'platform.baseline_plan_created': '生成采集计划',
  'platform.baseline_plan_downloaded': '下载采集计划',
  'platform.baseline_imported': '导入基准批次',
  'platform.baseline_batch_merged': '合并基准批次',
  'platform.baseline_batch_new_version': '以批次新建基准版本',
  'platform.baseline_batch_rejected': '拒绝基准批次',
  'platform.baseline_batch_excluded': '排除基准批次',
  'platform.baseline_version_activated': '启用基准版本',
  'model_catalog.created': '添加模型规则',
  'model_catalog.deleted': '删除模型规则',
  'model_catalog.updated': '修改模型规则',
  'platform.account_limits_updated': '修改帐号限额',
  'platform.email_settings_test_sent': '发送测试邮件',
  'platform.email_settings_updated': '修改邮箱设置',
  'platform.registration_updated': '修改注册设置',
  'platform.public_url_updated': '修改访问地址',
  'platform.public_verify_updated': '修改模型验证并发',
  'platform.account_disabled': '停用帐号',
  'platform.account_enabled': '恢复帐号',
  'platform.model_pricing_replaced': '批量设置模型价格',
  'platform.model_price_added': '添加模型价格',
  'platform.model_pricing_synced': '同步参考价',
  'platform.model_price_synced': '同步单个模型参考价',
  'route.created': '创建路由',
  'route.deleted': '删除路由',
  'virtual_key.created': '创建 API Key',
  'virtual_key.revealed': '查看 API Key',
  'virtual_key.revoked': '吊销 API Key',
  'virtual_key.updated': '修改 API Key',
  'virtual_key.usage_reset': '重置 API Key 用量',
}

const RESOURCE_LABELS: Record<string, string> = {
  account: '帐号',
  channel: '渠道',
  quality_monitor: '模型监控',
  virtual_key: 'API Key',
  model_catalog: '模型规则',
  model_pricing: '模型价格',
  system_settings: '系统设置',
  platform: '平台',
  route: '路由',
}

const PROTOCOL_LABELS: Record<string, string> = { openai: 'OpenAI', anthropic: 'Anthropic', gemini: 'Gemini' }

function formatMetadata(action: string, meta: Record<string, unknown>): string {
  const s = (k: string) => meta[k] as string | undefined
  const n = (k: string) => meta[k] as number | undefined
  const b = (k: string) => meta[k] as boolean | undefined

  switch (action) {
    case 'channel.created':
      return `协议：${PROTOCOL_LABELS[s('protocol') ?? ''] ?? s('protocol') ?? '未知'}`
    case 'channel.updated': {
      const parts: string[] = []
      if (b('models_updated')) parts.push('更新了模型')
      if (b('mapping_updated')) parts.push('更新了模型映射')
      if (b('enabled') === true) parts.push('已启用')
      if (b('enabled') === false) parts.push('已停用')
      return parts.join('，') || '—'
    }
    case 'channel.models_synced':
    case 'channel.models_updated':
      return n('count') != null ? `${n('count')} 个模型` : '—'
    case 'channel.connection_previewed': {
      const proto = PROTOCOL_LABELS[s('protocol') ?? ''] ?? s('protocol') ?? ''
      const ok = b('healthy')
      return [proto, ok === true ? '连接正常' : ok === false ? '连接失败' : ''].filter(Boolean).join('，')
    }
    case 'channel.monitor_run': {
      const kindLabel = s('kind') === 'quality' ? '质量探测' : '可用性探测'
      const statusLabel: Record<string, string> = { healthy: '健康', degraded: '降级', unhealthy: '异常', rate_limited: '限流', model_unavailable: '模型无效', unknown: '未知' }
      return `${kindLabel}：${statusLabel[s('status') ?? ''] ?? s('status') ?? '—'}`
    }
    case 'channel.monitor_updated': {
      const parts: string[] = []
      if (b('availability_enabled') != null) parts.push(`可用性监控：${b('availability_enabled') ? '开启' : '关闭'}`)
      if (b('quality_enabled') != null) parts.push(`质量监控：${b('quality_enabled') ? '开启' : '关闭'}`)
      return parts.join('，')
    }
    case 'quality_monitor.created': {
      const settings = meta.settings as Record<string, unknown> | undefined
      return `${s('model') ?? '模型'}，${settings?.schedule_enabled ? '定时开启' : '仅手动'}`
    }
    case 'quality_monitor.updated': {
      const policy = { observe: '仅观察', deprioritize: '异常时降低优先级', exclude: '异常时剔除' }[s('route_policy') ?? ''] ?? '—'
      return `${b('schedule_enabled') ? '定时开启' : '仅手动'}，${policy}`
    }
    case 'quality_monitor.run':
      return '检测已排队'
    case 'quality_monitor.deleted':
      return '停止后续检测，历史记录保留'
    case 'account.settings_updated':
      return s('api_key_prefix') ? `前缀：${s('api_key_prefix')}` : '—'
    case 'platform.account_limits_updated':
      return `渠道上限：${n('channel_limit') ?? '—'}，Key 上限：${n('virtual_key_limit') ?? '—'}`
    case 'platform.email_settings_updated':
      return s('host') ? `${s('host')}:${n('port')}` : '—'
    case 'platform.public_url_updated':
      return s('url') ?? '使用浏览器打开的地址'
    case 'platform.registration_updated': {
      const domains = (meta.allowed_domains as string[] | undefined) ?? []
      return `${b('enabled') ? '开放注册' : '关闭注册'}${domains.length ? `，限 ${domains.join('、')}` : ''}`
    }
    case 'platform.account_disabled':
      return [s('email'), s('reason') ? `原因：${s('reason')}` : ''].filter(Boolean).join('，') || '—'
    case 'platform.account_enabled':
      return s('email') ?? '—'
    case 'platform.model_pricing_replaced':
      return `${n('count') ?? 0} 个模型`
    case 'platform.model_price_added':
    case 'platform.model_price_synced':
      return s('pattern') ? `模型：${s('pattern')}` : '—'
    case 'platform.model_pricing_synced':
      return `已同步 ${n('synced') ?? 0} / ${n('requested') ?? 0} 个模型`
    case 'model_catalog.created':
    case 'model_catalog.updated':
    case 'model_catalog.deleted':
      return s('pattern') ? `规则：${s('pattern')}` : '—'
    default:
      return '—'
  }
}

function MetaSummary({ action, meta }: { action: string; meta: Record<string, unknown> }) {
  const text = formatMetadata(action, meta)
  return <span className="text-xs text-muted-foreground">{text}</span>
}

const columns: ColumnDef<AuditEvent, any>[] = [
  {
    accessorKey: 'created_at',
    header: '时间',
    enableSorting: true,
    cell: info => formatShortDate(info.getValue<string>()),
  },
  {
    accessorKey: 'action',
    header: '操作',
    cell: info => {
      const raw = info.getValue<string>()
      return <span>{ACTION_LABELS[raw] ?? raw}</span>
    },
  },
  {
    accessorKey: 'resource_type',
    header: '资源',
    cell: info => {
      const t = info.getValue<string | null>()
      return t ? <span>{RESOURCE_LABELS[t] ?? t}</span> : <span className="text-muted-foreground">—</span>
    },
  },
  {
    accessorKey: 'metadata',
    header: '详情',
    cell: info => <MetaSummary action={info.row.original.action} meta={info.getValue<Record<string, unknown>>()} />,
  },
]

export function ActivityPage() {
  const log = useAuditLog()
  return <>
    <PageHeader title="操作日志" description={CLOUD ? '当前帐号的操作记录，最近 200 条。' : '最近 200 条操作记录。'} />
    {log.isPending
      ? <Loading />
      : log.error
        ? <ErrorBox error={log.error} retry={() => log.refetch()} />
        : <DataTable
            data={log.data ?? []}
            columns={columns}
            empty={<Empty title="暂无操作记录" description="帐号下还没有任何可追踪的操作。" />}
          />}
  </>
}
