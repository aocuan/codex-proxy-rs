require 'tmpdir'
require 'uri'
require_relative 'package-release'
require_relative 'publish-release'

# 二开发行复用宿主的归档、清单和发布合同，只管理自身版本与镜像入口
module ForkRelease
  REPOSITORY = 'aocuan/codex-proxy-rs'
  IMAGE = "ghcr.io/#{REPOSITORY}"
  METADATA = 'dist/fork-release.json'

  def self.version(base, sequence, sha)
    base = ReleaseMetadata.version(base).split(/[+-]/, 2).first
    raise 'Invalid build sequence' unless sequence.match?(/\A[1-9]\d*\z/)
    raise 'Invalid source commit' unless sha.match?(/\A[0-9a-f]{40}\z/)
    ReleaseMetadata.version("#{base}-beta.#{sequence}+aocuan.#{sha[0, 12]}")
  end

  def self.metadata
    base = YAML.load_file('release/version.yaml').fetch('version')
    sha = ENV.fetch('GITHUB_SHA')
    value = version(base, ENV.fetch('GITHUB_RUN_NUMBER'), sha)
    data = {
      'version' => value, 'tag' => "v#{value}", 'upstream_version' => base,
      'docker_version' => value.tr('+', '-'), 'git_sha' => sha,
      'build_time' => Time.now.utc.iso8601, 'build_type' => 'release'
    }
    ReleaseMetadata.seal_manifest(data)
    File.write(METADATA, JSON.pretty_generate(data) + "\n")
    data.each { |key, entry| puts "#{key}=#{entry}" }
  end

  def self.package(architecture)
    data = JSON.parse(File.read(METADATA))
    platforms = ReleaseMetadata.platforms.select do |platform|
      platform['docker'] && platform['os'] == 'linux' && platform['arch'] == architecture
    end
    raise 'Unsupported fork release platform' unless platforms.length == 1
    compose = YAML.safe_load(File.read('deploy/compose.yaml'), aliases: true)
    app = compose.fetch('services').fetch('codex-proxy-rs')
    app['image'] = "${CPR_IMAGE:-#{IMAGE}:latest}"
    app.delete('build')
    app.fetch('environment')['CPR_UPDATE_REPOSITORY'] = REPOSITORY
    compose_path = 'dist/fork-compose.yaml'
    File.write(compose_path, YAML.dump(compose))
    ENV.update(
      'VERSION' => data.fetch('version'), 'DOCKER_VERSION' => data.fetch('docker_version'),
      'REGISTRY' => 'ghcr.io', 'IMAGE_NAME' => REPOSITORY,
      'PLATFORM_MATRIX' => JSON.generate('include' => platforms)
    )
    ReleasePackage.archives(compose_path: compose_path)
  end

  def self.collect(digest)
    raise 'Invalid image digest' unless digest.match?(/\Asha256:[a-f0-9]{64}\z/)
    data = JSON.parse(File.read(METADATA))
    target = 'dist/release'
    FileUtils.rm_rf(target)
    FileUtils.mkdir_p(target)
    roots = Dir.glob('dist/release-platforms/*').sort
    raise 'Expected both platform archives' unless roots.length == 2
    roots.each do |root|
      Dir.glob("#{root}/*").each do |path|
        next if File.basename(path) == 'checksums.txt'
        destination = File.join(target, File.basename(path))
        if File.exist?(destination) && File.binread(destination) != File.binread(path)
          raise "Platform packages disagree on #{File.basename(path)}"
        end
        FileUtils.cp(path, destination)
      end
    end
    expected = %w[amd64 arm64].map { |arch| "codex-proxy-rs_#{data.fetch('version')}_linux_#{arch}.tar.gz" }
    raise 'Missing or unexpected release archive' unless Dir.glob("#{target}/*.tar.gz").map { |path| File.basename(path) }.sort == expected.sort
    File.write("#{target}/image-digest.txt", "#{digest}\n")
    checksums = Dir.glob("#{target}/*").sort.map { |path| "#{Digest::SHA256.file(path).hexdigest}  #{File.basename(path)}\n" }
    File.write("#{target}/checksums.txt", checksums.join)
  end

  def self.can_promote?(target, current)
    return true if current.nil?
    comparison = ReleaseMetadata.sort_key(target) <=> ReleaseMetadata.sort_key(current)
    return true if comparison.positive? || (comparison.zero? && target == current)
    # 已部署的源码镜像没有发行通道，首次切换允许同一上游基线进入二开 Beta
    legacy = current.match(/\A(\d+\.\d+\.\d+)\+aocuan\.[0-9a-f]{12}\z/)
    legacy && ReleaseMetadata.sort_key(target.split('-', 2).first) >= ReleaseMetadata.sort_key(legacy[1])
  end

  def self.github_optional(endpoint)
    output, error, status = Open3.capture3('gh', 'api', '--include', endpoint)
    code = output.lines.first.to_s.split[1]
    return nil if code == '404'
    raise "Cannot inspect fork release: #{error.strip}" unless status.success? && code == '200'
    JSON.parse(output.split(/\r?\n\r?\n/, 2).fetch(1))
  end

  def self.ensure_tag(tag, sha)
    encoded = URI.encode_www_form_component(tag)
    unless github_optional("repos/#{REPOSITORY}/git/ref/tags/#{encoded}")
      ReleasePublication.run('gh', 'api', '--method', 'POST', "repos/#{REPOSITORY}/git/refs",
                             '-f', "ref=refs/tags/#{tag}", '-f', "sha=#{sha}")
    end
    actual = ReleasePublication.run('gh', 'api', "repos/#{REPOSITORY}/commits/#{encoded}", '--jq', '.sha')
    raise 'Release tag points to a different commit' unless actual == sha
  end

  def self.verify_published_assets(release, tag)
    expected = Dir.glob('dist/release/*').map { |path| File.basename(path) }.sort
    raise 'Published release assets differ; refusing to overwrite' unless release.fetch('assets').map { |asset| asset.fetch('name') }.sort == expected
    Dir.mktmpdir('cpr-release-check-') do |directory|
      ReleasePublication.run('gh', 'release', 'download', tag, '--repo', REPOSITORY,
                             '--pattern', 'checksums.txt', '--dir', directory)
      unless File.binread("#{directory}/checksums.txt") == File.binread('dist/release/checksums.txt')
        raise 'Published release bytes differ; refusing to overwrite'
      end
    end
  end

  def self.prepare
    data = JSON.parse(File.read(METADATA))
    tag = data.fetch('tag')
    notes = <<~MARKDOWN
      基于上游 #{data.fetch('upstream_version')} 的自研测评构建

      - 源码提交：[#{data.fetch('git_sha')[0, 12]}](https://github.com/#{REPOSITORY}/commit/#{data.fetch('git_sha')})
      - 包含账号质量记录、自定义测评、分钟计划与 TypeSafe 判题
      - Linux AMD64 / ARM64 镜像和在线更新归档使用同一套已验证程序、前端与发行清单

      已使用二开 Beta 镜像的实例可在系统更新中检查并点击升级，安装后需重启
      `source` 构建首次接入需按[部署说明](https://github.com/#{REPOSITORY}/blob/#{data.fetch('git_sha')}/deploy/README.md#自研测评镜像)更换镜像，不能从旧版页面直接升级

      镜像：`#{IMAGE}:#{data.fetch('docker_version')}`
      本发行线不更新上游仓库、GitHub Latest 或镜像 latest 标签
    MARKDOWN
    File.write('dist/fork-release-notes.md', notes)
    ensure_tag(tag, data.fetch('git_sha'))
    # 草稿创建与附件上传复用上游发行 action，直接消费它返回的 ID，避免立即读回草稿
    release = github_optional("repos/#{REPOSITORY}/releases/tags/#{URI.encode_www_form_component(tag)}")
    return nil unless release
    raise 'Fork release must remain a prerelease' unless release.fetch('prerelease')
    return nil if release.fetch('draft')
    verify_published_assets(release, tag)
    release.fetch('id').to_s
  end

  def self.publish(digest)
    data = JSON.parse(File.read(METADATA))
    ReleasePublication.publish(version: data.fetch('version'), image: IMAGE, digest: digest,
                               repository: REPOSITORY, release_id: ENV.fetch('RELEASE_ID'))
    current = ReleasePublication.latest_image(IMAGE, 'evaluation')
    if can_promote?(data.fetch('version'), current)
      ReleasePublication.tag(IMAGE, digest, 'evaluation')
    else
      puts "Published #{data.fetch('tag')}; evaluation remains at #{current}"
    end
  end
end

if $PROGRAM_NAME == __FILE__
  begin
    case ARGV
    when ['metadata'] then ForkRelease.metadata
    when ['prepare'] then puts "release_id=#{ForkRelease.prepare}"
    else
      command, value = ARGV
      raise 'usage: fork-release.rb metadata|prepare|package ARCH|collect DIGEST|publish DIGEST' unless ARGV.length == 2
      case command
      when 'package' then ForkRelease.package(value)
      when 'collect' then ForkRelease.collect(value)
      when 'publish' then ForkRelease.publish(value)
      else raise 'Unknown fork release command'
      end
    end
  rescue StandardError => error
    warn "error: #{error.message}"
    exit 1
  end
end
