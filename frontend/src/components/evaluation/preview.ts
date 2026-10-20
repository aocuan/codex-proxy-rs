import type { CssNode, EnterOrLeaveFn } from 'css-tree'
import { generate, parse, walk } from 'css-tree'
import DOMPurify from 'dompurify'

// 只保留绘图与 CSS 动画所需的声明，网络、脚本、导航另由 sandbox 和 CSP 隔离
const properties = new Set(`
  color background background-color background-image opacity
  width height min-width min-height max-width max-height aspect-ratio
  margin margin-top margin-right margin-bottom margin-left
  padding padding-top padding-right padding-bottom padding-left
  display position top right bottom left inset overflow overflow-x overflow-y
  box-sizing border border-width border-style border-color border-radius box-shadow
  font font-family font-size font-weight font-style line-height letter-spacing
  text-align text-anchor text-decoration white-space vertical-align
  align-items align-content justify-content justify-items place-items gap row-gap column-gap
  flex flex-direction flex-wrap flex-grow flex-shrink flex-basis order
  grid-template-columns grid-template-rows grid-column grid-row
  transform transform-origin transform-box perspective backface-visibility
  animation animation-name animation-duration animation-timing-function animation-delay
  animation-iteration-count animation-direction animation-fill-mode animation-play-state
  transition transition-property transition-duration transition-delay transition-timing-function
  fill fill-opacity fill-rule stroke stroke-width stroke-opacity stroke-linecap stroke-linejoin
  stroke-dasharray stroke-dashoffset stroke-miterlimit clip-path clip-rule filter
  stop-color stop-opacity flood-color flood-opacity color-interpolation-filters
  vector-effect paint-order dominant-baseline visibility isolation mix-blend-mode
`.trim().split(/\s+/))

const functions = new Set(`
  var calc min max clamp rgb rgba hsl hsla hwb lab lch oklab oklch color color-mix
  translate translatex translatey translatez translate3d scale scalex scaley scalez scale3d
  rotate rotatex rotatey rotatez rotate3d skew skewx skewy matrix matrix3d perspective
  cubic-bezier steps linear
  linear-gradient radial-gradient conic-gradient repeating-linear-gradient repeating-radial-gradient
  blur brightness contrast drop-shadow grayscale hue-rotate invert opacity saturate sepia
  circle ellipse inset polygon path
`.trim().split(/\s+/))

function safeValue(value: CssNode) {
  let safe = true
  walk(value, (node) => {
    if (node.type === 'Raw'
      || (node.type === 'Url' && !/^#[\w-]+$/.test(node.value))
      || (node.type === 'Function' && !functions.has(node.name.toLowerCase()))) {
      safe = false
    }
  })
  return safe
}

function cleanCss(source: string, context: 'stylesheet' | 'declarationList') {
  // 序列化 style 时不能让原始文本重新关闭元素
  if (/[<>]/.test(source))
    return ''
  try {
    const ast = parse(source, {
      context,
      parseCustomProperty: true,
      onParseError: () => {
        throw new Error('CSS 无法解析')
      },
    })
    const visitor: EnterOrLeaveFn = function (node, item, list) {
      if (node.type === 'Atrule' && (!['keyframes', '-webkit-keyframes', 'media'].includes(node.name.toLowerCase()) || !node.block)) {
        list?.remove(item)
        return this.skip
      }
      if (node.type === 'Declaration'
        && ((!properties.has(node.property.toLowerCase()) && !/^--[\w-]+$/.test(node.property))
          || !safeValue(node.value))) {
        list?.remove(item)
        return this.skip
      }
    }
    walk(ast, visitor)
    return generate(ast)
  }
  catch {
    return ''
  }
}

export interface SafeEvaluationPreview {
  srcdoc: string
}

export function createEvaluationPreview(html: string): SafeEvaluationPreview {
  if (new TextEncoder().encode(html).byteLength > 256 * 1024)
    throw new Error('作品超过预览大小限制')

  const documentElement = DOMPurify.sanitize(html, {
    WHOLE_DOCUMENT: true,
    RETURN_DOM: true,
    USE_PROFILES: { html: true, svg: true, svgFilters: true },
    FORBID_TAGS: [
      'script',
      'iframe',
      'frame',
      'frameset',
      'object',
      'embed',
      'base',
      'link',
      'meta',
      'a',
      'form',
      'input',
      'button',
      'textarea',
      'select',
      'option',
      'audio',
      'video',
      'source',
      'track',
      'img',
      'image',
      'feImage',
      'foreignObject',
      'animate',
      'animateMotion',
      'animateTransform',
      'set',
      'mpath',
    ],
    FORBID_ATTR: ['src', 'srcset', 'action', 'formaction', 'poster', 'background', 'ping', 'target', 'download', 'nonce', 'autofocus'],
    ALLOW_DATA_ATTR: false,
  })
  if (!(documentElement instanceof HTMLElement))
    throw new Error('作品无法解析为 HTML 页面')
  const nodes = documentElement.querySelectorAll('*')
  if (nodes.length > 10000)
    throw new Error('作品元素过多，无法安全预览')
  for (const element of nodes) {
    if (element.localName === 'style') {
      const before = element.textContent ?? ''
      const after = cleanCss(before, 'stylesheet')
      element.textContent = after
    }
    for (const attribute of Array.from(element.attributes)) {
      const name = attribute.name.toLowerCase()
      if (name === 'style') {
        const value = cleanCss(attribute.value, 'declarationList')
        element.setAttribute('style', value)
      }
      else if ((name === 'href' || name === 'xlink:href') && !(element.localName === 'use' && /^#[\w-]+$/.test(attribute.value))) {
        element.removeAttribute(attribute.name)
      }
      else if (['fill', 'stroke', 'filter', 'clip-path', 'mask', 'marker-start', 'marker-mid', 'marker-end'].includes(name)) {
        try {
          if (!safeValue(parse(attribute.value, { context: 'value' }))) {
            element.removeAttribute(attribute.name)
          }
        }
        catch {
          element.removeAttribute(attribute.name)
        }
      }
    }
  }
  const head = documentElement.querySelector('head')
  if (!head || !documentElement.querySelector('svg'))
    throw new Error('作品缺少可预览的 SVG')
  const csp = document.createElement('meta')
  csp.httpEquiv = 'Content-Security-Policy'
  csp.content = 'default-src \'none\'; script-src \'none\'; style-src \'unsafe-inline\'; img-src \'none\'; font-src \'none\'; connect-src \'none\'; frame-src \'none\'; object-src \'none\'; media-src \'none\'; base-uri \'none\'; form-action \'none\''
  head.prepend(csp)
  return { srcdoc: `<!doctype html>${documentElement.outerHTML}` }
}
