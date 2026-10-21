require 'minitest/autorun'
require_relative '../fork-release'

# 发行测试覆盖更新顺序、镜像首次迁移和归档字节一致性
class ForkReleaseTest < Minitest::Test
  ROOT = File.expand_path('../../..', __dir__)
  SHA = 'a' * 40

  def in_workspace
    Dir.mktmpdir('cpr-fork-release-') do |directory|
      Dir.chdir(directory) do
        FileUtils.mkdir_p(%w[deploy release dist])
        %w[deploy/compose.yaml deploy/config.example.yaml release/platforms.yaml].each do |path|
          FileUtils.cp(File.join(ROOT, path), path)
        end
        begin
          yield
        ensure
          FileUtils.chmod_R('u+w', 'package') if File.directory?('package')
        end
      end
    end
  end

  def metadata
    { 'version' => ForkRelease.version('3.29.7', '1', SHA), 'git_sha' => SHA,
      'docker_version' => ForkRelease.version('3.29.7', '1', SHA).tr('+', '-') }
  end

  def test_build_sequence_changes_semver_precedence_without_changing_upstream_files
    first = ForkRelease.version('v3.29.7', '1', SHA)
    second = ForkRelease.version('v3.29.7', '2', 'b' * 40)
    assert_equal '3.29.7-beta.1+aocuan.aaaaaaaaaaaa', first
    assert_operator ReleaseMetadata.sort_key(second), :>, ReleaseMetadata.sort_key(first)
    assert_operator ReleaseMetadata.sort_key(ForkRelease.version('3.29.8', '3', SHA)), :>, ReleaseMetadata.sort_key(second)
    assert_raises(RuntimeError) { ForkRelease.version('3.29.7', '0', SHA) }
    assert_raises(RuntimeError) { ForkRelease.version('3.29.7', '01', SHA) }
    assert_raises(RuntimeError) { ForkRelease.version('3.29.7', '1', 'not-a-commit') }
  end

  def test_evaluation_pointer_never_rolls_back_and_accepts_legacy_source_bootstrap
    first = ForkRelease.version('3.29.7', '1', SHA)
    second = ForkRelease.version('3.29.7', '2', SHA)
    assert ForkRelease.can_promote?(first, nil)
    assert ForkRelease.can_promote?(first, '3.29.7+aocuan.bbbbbbbbbbbb')
    assert ForkRelease.can_promote?(second, first)
    assert ForkRelease.can_promote?(second, second)
    refute ForkRelease.can_promote?(first, second)
    refute ForkRelease.can_promote?(first, '3.29.8+aocuan.bbbbbbbbbbbb')
    refute ForkRelease.can_promote?(first, '3.29.7')
    refute ForkRelease.can_promote?(first, ForkRelease.version('3.29.7', '1', 'b' * 40))
  end

  def test_archives_preserve_verified_files_and_publish_matching_checksums
    previous_env = ENV.to_h
    in_workspace do
      File.write(ForkRelease::METADATA, JSON.generate(metadata))
      FileUtils.mkdir_p('dist/plugin-release-assets')
      manifest = { schema_version: 1, sealed: true, gateway_version: metadata['version'], gateway_git_sha: SHA, plugins: [] }
      File.write(ReleasePackage::MANIFEST, JSON.generate(manifest))
      %w[amd64 arm64].each do |arch|
        directory = "dist/binaries/codex-proxy-rs-linux-#{arch}"
        FileUtils.mkdir_p([directory, 'dist/frontend'])
        File.write("#{directory}/codex-proxy-rs", "verified-binary-#{arch}")
        File.write('dist/frontend/index.html', '<html>verified frontend</html>')
        ForkRelease.package(arch)
        archive = "dist/release/codex-proxy-rs_#{metadata['version']}_linux_#{arch}.tar.gz"
        files, status = Open3.capture2('tar', '-tzf', archive)
        assert status.success?
        assert_includes files, 'codex-proxy-rs/plugins/official/plugin-release-manifest.json'
        binary, status = Open3.capture2('tar', '-xOzf', archive, 'codex-proxy-rs/codex-proxy-rs')
        assert status.success?
        assert_equal "verified-binary-#{arch}", binary
        compose = YAML.safe_load(File.read('dist/release/compose.yaml'), aliases: true)
        app = compose.fetch('services').fetch('codex-proxy-rs')
        assert_equal ForkRelease::REPOSITORY, app.fetch('environment').fetch('CPR_UPDATE_REPOSITORY')
        assert_includes app.fetch('image'), metadata.fetch('docker_version')
        refute app.key?('build')
        destination = "dist/release-platforms/#{arch}"
        FileUtils.mkdir_p(destination)
        FileUtils.cp_r('dist/release/.', destination)
      end
      ForkRelease.collect('sha256:' + 'b' * 64)
      File.readlines('dist/release/checksums.txt').each do |line|
        hash, name = line.split
        assert_equal hash, Digest::SHA256.file("dist/release/#{name}").hexdigest
      end
      File.write('dist/release-platforms/arm64/plugin-release-manifest.json', '{}')
      assert_raises(RuntimeError) { ForkRelease.collect('sha256:' + 'b' * 64) }
    end
  ensure
    ENV.replace(previous_env)
  end

  def test_published_release_with_different_assets_is_never_overwritten
    in_workspace do
      FileUtils.mkdir_p('dist/release')
      File.write('dist/release/checksums.txt', 'expected')
      assert_raises(RuntimeError) do
        ForkRelease.verify_published_assets({ 'assets' => [] }, 'v3.29.7-beta.1')
      end
    end
  end

  def test_tag_is_created_at_the_build_commit_before_draft_publication
    calls = []
    runner = lambda do |*args|
      calls << args
      args.include?('.sha') ? SHA : '{}'
    end
    ForkRelease.stub(:github_optional, nil) do
      ReleasePublication.stub(:run, runner) { ForkRelease.ensure_tag('v3.29.7-beta.1', SHA) }
    end
    assert_includes calls.first, "sha=#{SHA}"
    assert_includes calls.first, 'ref=refs/tags/v3.29.7-beta.1'
    assert_equal 2, calls.length
    ForkRelease.stub(:github_optional, {}) do
      ReleasePublication.stub(:run, 'b' * 40) do
        assert_raises(RuntimeError) { ForkRelease.ensure_tag('v3.29.7-beta.1', SHA) }
      end
    end
  end
end
