import { WebGLTexture } from './texture'

export class Device {
  public get backendName (): string {
    return 'WebGL'
  }
  public deviceName (): string {
    return 'WebGL Device'
  }

  createTexture (format: TextureFormat, size: number) {
    const glTexture = this.context.createTexture()
    const texture = WebGLTexture.create(context, glTexture, format, size)
    
    this.bindTexture({

    })
  }

  createTextureFromData () {}
  createShaderFromSource () {}
  createProgramFromShaders () {}
  createShader () {}
}






